import type { ChatAttachment } from "./types.ts";
import { EditFailed, type AttachmentEdit } from "./edit.ts";
import {
  compressionSettings,
  DEFAULT_COMPRESSION,
  prepareFile,
  type CompressionSettings,
  type PreparedFile,
} from "./prepare.ts";
import { voiceFilename, type VoiceRecording } from "./voice.ts";

export const MAX_ATTACHMENTS = 10;
/** Allow a queued batch of ten files (two minutes per original/preview) to finish cleaning. */
export const COMPLETE_RETRY_DELAYS_MS = [250, 500, 1_000, 2_000, ...Array<number>(240).fill(5_000)];
/** Parts of a large file in flight at once, and part URLs asked for at a time.
 * Asking as parts go also keeps a long upload alive on the server. */
const PART_CONCURRENCY = 4;
const PART_BATCH = 4;
/** A failed part is sent again with a fresh URL after each of these pauses. */
export const PART_RETRY_DELAYS_MS = [500, 2_000, 5_000, 15_000];

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

export interface UploadTransport {
  fetch: typeof fetch;
  /** PUT with progress; browsers use XMLHttpRequest because fetch cannot report upload progress. */
  put: (
    url: string,
    headers: Record<string, string>,
    body: Blob,
    progress: (fraction: number) => void,
    signal: AbortSignal,
  ) => Promise<void>;
  /** Pause between `/complete` retries; injectable so tests need not wait. */
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
}

export class UploadError extends Error {
  readonly storageFull: boolean;
  constructor(message: string, storageFull = false) {
    super(message);
    this.storageFull = storageFull;
  }
}

export interface UploadState {
  name: string;
  sourceSize: number;
  storedSize?: number;
  compressing?: number;
  progress: number;
  error?: string;
  attachment?: ChatAttachment;
}

/** Own the local bytes until removal or send confirmation. Focus, Send and Retry
 * share one upload, and renew well before the server's 24-hour unsent lifetime.
 * The original file is kept, so edits always start from it. */
export class AttachmentUpload {
  state: UploadState;
  private file?: File;
  private edits?: AttachmentEdit;
  private prepared?: PreparedFile;
  private completedAt?: number;
  private pending?: Promise<ChatAttachment>;
  private controller = new AbortController();

  constructor(
    private channelId: string,
    file: File,
    private settings: CompressionSettings,
    private changed: (state: UploadState) => void,
    private transport: UploadTransport = browserTransport,
    private now: () => number = Date.now,
  ) {
    this.file = file;
    this.state = { name: file.name, sourceSize: file.size, progress: 0 };
  }

  /** A recorded voice message: uploaded exactly as recorded, with its duration
   * and waveform, and no preparation or compression. */
  static voice(
    channelId: string,
    recording: VoiceRecording,
    changed: (state: UploadState) => void = () => {},
    transport: UploadTransport = browserTransport,
    now: () => number = Date.now,
  ) {
    const name = voiceFilename(recording.contentType);
    const file = new File([recording.blob], name, { type: recording.contentType });
    const upload = new AttachmentUpload(channelId, file, DEFAULT_COMPRESSION, changed, transport, now);
    upload.prepared = voicePreparedFile(recording);
    return upload;
  }

  get stale() {
    return (
      !!this.state.attachment &&
      (this.completedAt === undefined || this.now() - this.completedAt >= 23 * 60 * 60 * 1_000)
    );
  }

  invalidate() {
    this.completedAt = undefined;
  }

  /** The original file, for the editor; unset once released. */
  get source() {
    return this.file;
  }

  get edit() {
    return this.edits;
  }

  /** Replaces the edits: whatever was prepared or uploaded is cancelled, and
   * the original is prepared and uploaded again with the new edits. */
  setEdit(edit: AttachmentEdit | undefined): Promise<ChatAttachment> {
    if (!this.file) return Promise.reject(new UploadError("This file is no longer available."));
    this.controller.abort();
    this.controller = new AbortController();
    if (this.state.attachment) void cancelUpload(this.state.attachment.id, this.transport.fetch);
    this.edits = edit;
    this.prepared = undefined;
    this.completedAt = undefined;
    this.pending = undefined;
    this.update({
      name: this.file.name,
      storedSize: undefined,
      compressing: undefined,
      attachment: undefined,
      error: undefined,
      progress: 0,
    });
    return this.ensure();
  }

  ensure(): Promise<ChatAttachment> {
    if (this.pending) return this.pending;
    if (this.state.attachment && !this.stale) return Promise.resolve(this.state.attachment);
    const pending = this.upload().finally(() => {
      // An edit may have started a newer upload meanwhile.
      if (this.pending === pending) this.pending = undefined;
    });
    this.pending = pending;
    return pending;
  }

  release(cancel = false) {
    this.controller.abort();
    if (cancel && this.state.attachment) void cancelUpload(this.state.attachment.id, this.transport.fetch);
    this.file = undefined;
    this.prepared = undefined;
  }

  private update(change: Partial<UploadState>) {
    this.state = { ...this.state, ...change };
    this.changed(this.state);
  }

  private async upload(): Promise<ChatAttachment> {
    const signal = this.controller.signal;
    signal.throwIfAborted();
    if (this.state.attachment) void cancelUpload(this.state.attachment.id, this.transport.fetch);
    this.update({ attachment: undefined, error: undefined, progress: 0 });
    try {
      const prepared =
        this.prepared ??
        (await prepareFile(
          this.file!,
          this.settings,
          (compressing) => {
            if (!signal.aborted) this.update({ compressing });
          },
          signal,
          this.edits,
        ));
      // A superseded (edited or removed) preparation must not be kept for retries.
      signal.throwIfAborted();
      this.prepared = prepared;
      this.update({ name: prepared.name, storedSize: prepared.blob.size, compressing: undefined });
      const attachment = await uploadPrepared(
        this.channelId,
        prepared,
        this.transport,
        (progress) => {
          if (!signal.aborted) this.update({ progress });
        },
        signal,
      );
      this.completedAt = this.now();
      this.update({ attachment, progress: 1 });
      return attachment;
    } catch (error) {
      if (!signal.aborted)
        this.update({
          error: error instanceof UploadError || error instanceof EditFailed ? error.message : "Failed to upload.",
        });
      throw error;
    }
  }
}

export function voicePreparedFile(recording: VoiceRecording): PreparedFile {
  return {
    blob: recording.blob,
    name: voiceFilename(recording.contentType),
    contentType: recording.contentType,
    kind: "audio",
    sourceSize: recording.blob.size,
    durationMs: recording.durationMs,
    voice: true,
    waveform: recording.waveform || undefined,
  };
}

interface PresignedPut {
  method: "PUT";
  url: string;
  headers: Record<string, string>;
}

interface MultipartPlan {
  partBytes: number;
  partCount: number;
}

async function failure(response: Response, fallback: string) {
  const body = (await response.json().catch(() => undefined)) as { error?: unknown; code?: unknown } | undefined;
  if (body?.code === "storage_full") return new UploadError("You’ve used all of your file storage.", true);
  if (body?.code === "media_too_large") return new UploadError("Photos, videos and audio can be up to 5 GB.");
  if (body?.code === "upload_incomplete") return new UploadError("The upload did not arrive intact. Try again.");
  if (response.status === 413) return new UploadError("This file is too large to upload.");
  if (response.status === 422 && body?.code !== "media_metadata")
    return new UploadError("The upload did not arrive intact. Try again.");
  return new UploadError(typeof body?.error === "string" ? body.error : fallback);
}

export function sleep(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason);
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", abort);
      resolve();
    }, ms);
    const abort = () => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    signal.addEventListener("abort", abort, { once: true });
  });
}

/** Best effort; abandonment cleanup remains the fallback for offline clients. */
export async function cancelUpload(id: string, fetcher: typeof fetch = fetch): Promise<void> {
  try {
    await fetcher(`/api/assets/${encodeURIComponent(id)}/cancel`, {
      method: "POST",
      credentials: "same-origin",
      keepalive: true,
      signal: AbortSignal.timeout(5_000),
    });
  } catch {
    // Do not turn a removed file into a composer error.
  }
}

/** Send a large file in parts, each straight to storage: a few at a time, URLs
 * asked for as they are needed, and a failed part sent again with a fresh URL.
 * The first part that can't be sent stops the rest. */
async function uploadParts(
  id: string,
  blob: Blob,
  plan: MultipartPlan,
  transport: UploadTransport,
  progress: (fraction: number) => void,
  signal: AbortSignal,
) {
  const stop = new AbortController();
  const abort = () => stop.abort(signal.reason);
  signal.addEventListener("abort", abort, { once: true });
  const local = stop.signal;
  const sent = Array.from({ length: plan.partCount }, () => 0);
  const report = () => progress(sent.reduce((sum, bytes) => sum + bytes, 0) / blob.size);
  const urls = new Map<number, string>();
  const requested = new Map<number, Promise<void>>();
  const request = async (numbers: number[]) => {
    const response = await transport.fetch(`/api/assets/${encodeURIComponent(id)}/parts`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      credentials: "same-origin",
      signal: local,
      body: JSON.stringify({ parts: numbers }),
    });
    if (!response.ok) throw await failure(response, "This file could not be uploaded.");
    const body = (await response.json()) as { parts?: Array<{ number?: unknown; url?: unknown }> };
    for (const part of body.parts ?? [])
      if (typeof part.number === "number" && typeof part.url === "string") urls.set(part.number, part.url);
  };
  const urlFor = async (number: number) => {
    if (!urls.has(number)) {
      if (!requested.has(number)) {
        const batch: number[] = [];
        for (let next = number; next <= plan.partCount && batch.length < PART_BATCH; next++)
          if (!urls.has(next) && !requested.has(next)) batch.push(next);
        const pending = request(batch).finally(() => batch.forEach((part) => requested.delete(part)));
        for (const part of batch) requested.set(part, pending);
      }
      await requested.get(number);
    }
    const url = urls.get(number);
    if (!url) throw new UploadError("The upload service returned an invalid response.");
    return url;
  };
  const send = async (number: number) => {
    const body = blob.slice((number - 1) * plan.partBytes, Math.min(number * plan.partBytes, blob.size));
    for (let attempt = 0; ; attempt++) {
      const url = await urlFor(number);
      urls.delete(number);
      try {
        await transport.put(
          url,
          {},
          body,
          (fraction) => {
            sent[number - 1] = fraction * body.size;
            report();
          },
          local,
        );
        sent[number - 1] = body.size;
        report();
        return;
      } catch (error) {
        if (local.aborted || attempt >= PART_RETRY_DELAYS_MS.length) throw error;
        sent[number - 1] = 0;
        report();
        await (transport.sleep ?? sleep)(PART_RETRY_DELAYS_MS[attempt], local);
      }
    }
  };
  let next = 1;
  const worker = async () => {
    while (next <= plan.partCount) await send(next++);
  };
  try {
    await Promise.all(Array.from({ length: Math.min(PART_CONCURRENCY, plan.partCount) }, worker));
  } catch (error) {
    stop.abort(error);
    throw signal.aborted ? signal.reason : error;
  } finally {
    signal.removeEventListener("abort", abort);
  }
}

/** Reserve the prepared (compressed) file and its preview, PUT each straight
 * to storage with exactly the signed headers (preview first), then confirm.
 * Files larger than one part go up in parts instead of one PUT.
 * Returns the attachment description to send with a message. */
export async function uploadPrepared(
  channelId: string,
  file: PreparedFile,
  transport: UploadTransport,
  progress: (fraction: number) => void,
  signal: AbortSignal,
): Promise<ChatAttachment> {
  signal.throwIfAborted();
  const created = await transport.fetch("/api/assets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    // Receive the reservation even if removal races this request, so we can
    // cancel its ID rather than silently abandoning a successful reservation.
    signal: AbortSignal.timeout(15_000),
    body: JSON.stringify({
      ...(file.voice ? { purpose: "attachment" } : {}),
      channelId,
      filename: file.name,
      contentType: file.contentType || "application/octet-stream",
      byteSize: file.blob.size,
      sourceByteSize: file.sourceSize || undefined,
      width: file.width,
      height: file.height,
      durationMs: file.durationMs,
      preview: file.preview ? { contentType: file.preview.type, byteSize: file.preview.size } : undefined,
      ...(file.voice ? { voice: true, waveform: file.waveform } : {}),
      multipart: true,
    }),
  });
  if (!created.ok) throw await failure(created, "This file could not be uploaded.");
  const reservation = (await created.json()) as {
    id?: unknown;
    upload?: Partial<PresignedPut>;
    multipart?: Partial<MultipartPlan>;
    previewUpload?: Partial<PresignedPut>;
  };
  if (typeof reservation.id !== "string") throw new UploadError("The upload service returned an invalid response.");
  try {
    signal.throwIfAborted();
    const plan = reservation.multipart;
    const parts =
      plan && Number.isSafeInteger(plan.partBytes) && Number.isSafeInteger(plan.partCount) && plan.partBytes! > 0
        ? { partBytes: plan.partBytes!, partCount: plan.partCount! }
        : undefined;
    if (
      plan
        ? !parts || parts.partCount !== Math.ceil(file.blob.size / parts.partBytes)
        : typeof reservation.upload?.url !== "string"
    )
      throw new UploadError("The upload service returned an invalid response.");
    // A reserved preview must be uploaded, or /complete rejects the asset.
    if (file.preview && typeof reservation.previewUpload?.url !== "string")
      throw new UploadError("The upload service returned an invalid response.");
    const preview =
      file.preview && reservation.previewUpload?.url
        ? { blob: file.preview, url: reservation.previewUpload.url, headers: reservation.previewUpload.headers ?? {} }
        : undefined;
    const previewSize = preview?.blob.size ?? 0;
    const total = file.blob.size + previewSize;
    if (preview)
      await transport.put(
        preview.url,
        preview.headers,
        preview.blob,
        (fraction) => progress((fraction * previewSize) / total),
        signal,
      );
    const report = (fraction: number) => progress((previewSize + fraction * file.blob.size) / total);
    if (parts) await uploadParts(reservation.id, file.blob, parts, transport, report, signal);
    else await transport.put(reservation.upload!.url!, reservation.upload!.headers ?? {}, file.blob, report, signal);
    const path = `/api/assets/${encodeURIComponent(reservation.id)}/complete`;
    for (let attempt = 0; ; attempt++) {
      const completed = await transport.fetch(path, { method: "POST", credentials: "same-origin", signal });
      if (completed.ok) {
        const attachment = (await completed.json()) as ChatAttachment;
        signal.throwIfAborted();
        progress(1);
        return attachment;
      }
      // 409: storage is not visible yet, or metadata processing is queued.
      // Missing parts won't arrive by waiting.
      const incomplete =
        completed.status === 409 &&
        (
          (await completed
            .clone()
            .json()
            .catch(() => undefined)) as { code?: unknown } | undefined
        )?.code === "upload_incomplete";
      if (completed.status !== 409 || incomplete || attempt >= COMPLETE_RETRY_DELAYS_MS.length) {
        throw await failure(completed, "This file could not be uploaded.");
      }
      await completed.body?.cancel().catch(() => undefined);
      await (transport.sleep ?? sleep)(COMPLETE_RETRY_DELAYS_MS[attempt], signal);
    }
  } catch (error) {
    await cancelUpload(reservation.id, transport.fetch);
    throw error;
  }
}

export const browserTransport: UploadTransport = {
  fetch: (...args) => fetch(...args),
  put: (url, headers, body, progress, signal) =>
    new Promise((resolve, reject) => {
      signal.throwIfAborted();
      const request = new XMLHttpRequest();
      request.open("PUT", url);
      // Exactly the signed headers; the browser adds the matching content-length.
      for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
      request.upload.onprogress = (event) => {
        if (event.lengthComputable) progress(event.loaded / event.total);
      };
      request.onload = () =>
        request.status >= 200 && request.status < 300
          ? resolve()
          : reject(new UploadError("Storage refused the upload."));
      request.onerror = () => reject(new UploadError("The upload was interrupted."));
      request.onabort = () => reject(new DOMException("Upload cancelled.", "AbortError"));
      signal.addEventListener("abort", () => request.abort(), { once: true });
      request.send(body);
    }),
};

/** Uploads are optional server configuration: undefined disables the control.
 * Otherwise returns the server's compression settings from `/api/assets/usage`. */
export async function uploadSettings(fetcher: typeof fetch = fetch): Promise<CompressionSettings | undefined> {
  const response = await fetcher("/api/assets/usage", { credentials: "same-origin", cache: "no-store" }).catch(
    () => undefined,
  );
  if (!response?.ok) return;
  const body = (await response.json().catch(() => undefined)) as { compression?: unknown } | undefined;
  return compressionSettings(body?.compression);
}

/** Fresh URLs for attachments whose signed URLs expired in a long-open tab.
 * Only ready attachments have a `url`. */
export async function refreshAttachmentUrls(
  ids: string[],
  fetcher: typeof fetch = fetch,
): Promise<Record<string, { url?: string; previewUrl?: string }>> {
  // Offline is no fresh URLs, like a refusal; callers fire and forget.
  const response = await fetcher("/api/assets/urls", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ ids }),
  }).catch(() => undefined);
  if (!response?.ok) return {};
  const body = (await response.json().catch(() => undefined)) as
    | { urls?: Record<string, { url?: unknown; previewUrl?: unknown }> }
    | undefined;
  const safe = (value: unknown) => (typeof value === "string" && /^https?:\/\//.test(value) ? value : undefined);
  const urls: Record<string, { url?: string; previewUrl?: string }> = {};
  for (const [id, entry] of Object.entries(body?.urls && typeof body.urls === "object" ? body.urls : {})) {
    const url = safe(entry?.url),
      previewUrl = safe(entry?.previewUrl);
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
