// Preparing and uploading a profile picture or space icon: the same three
// steps as attachments (reserve, presigned PUT, complete) with a `purpose`.
// The server applies the picture when the upload completes.
import { encode, surface, type Context2D, type Surface } from "../chat/prepare.ts";
import {
  browserTransport,
  cancelUpload,
  COMPLETE_RETRY_DELAYS_MS,
  sleep,
  type UploadTransport,
} from "../chat/uploads.ts";
import type { Space } from "../spaces/client.ts";
import { rememberSavedAccount, type Account } from "./client.ts";
import { GifShrinkError, shrinkGif } from "./gif-shrink.ts";
import {
  gifSize,
  gifUploadsUnchanged,
  MAX_PICTURE_BYTES,
  pictureEdge,
  pictureErrorMessage,
  pictureFilename,
  pictureMessages,
  squareCrop,
  type PicturePurpose,
} from "./pictures.ts";

/** WebP quality for re-encoded pictures. */
const PICTURE_QUALITY = 0.9;

/** A message ready to show. `status` is the API's, when it answered; 503 means uploads are off. */
export class PictureError extends Error {
  readonly status?: number;
  constructor(message: string, status?: number) {
    super(message);
    this.status = status;
  }
}

export interface PreparedPicture {
  blob: Blob;
  contentType: string;
  width: number;
  height: number;
}

interface Decoded {
  source: CanvasImageSource;
  width: number;
  height: number;
  close: () => void;
}

/** EXIF orientation applied; HEIC and other formats only where the browser can decode them. */
async function decode(file: Blob): Promise<Decoded> {
  if (typeof createImageBitmap !== "undefined") {
    const bitmap = await createImageBitmap(file, { imageOrientation: "from-image" }).catch(() => undefined);
    if (bitmap) return { source: bitmap, width: bitmap.width, height: bitmap.height, close: () => bitmap.close() };
  }
  // Some browsers decode more formats (or SVG) through an image element than createImageBitmap.
  if (typeof Image === "undefined") throw new PictureError(pictureMessages.undecodable);
  const url = URL.createObjectURL(file);
  const image = new Image();
  image.src = url;
  try {
    await image.decode();
  } catch {
    URL.revokeObjectURL(url);
    throw new PictureError(pictureMessages.undecodable);
  }
  if (!image.naturalWidth || !image.naturalHeight) {
    URL.revokeObjectURL(url);
    throw new PictureError(pictureMessages.undecodable);
  }
  return {
    source: image,
    width: image.naturalWidth,
    height: image.naturalHeight,
    close: () => URL.revokeObjectURL(url),
  };
}

/** PNG when WebP can't be encoded (it keeps transparency), else JPEG on the app's surface colour. */
async function fallback(canvas: Surface) {
  const png = await encode(canvas, "image/png");
  if (png && png.size <= MAX_PICTURE_BYTES) return png;
  const flat = surface(canvas.width, canvas.height);
  const context = flat.getContext("2d") as Context2D | null;
  if (!context) return png;
  context.fillStyle = "#151719";
  context.fillRect(0, 0, flat.width, flat.height);
  context.drawImage(canvas, 0, 0);
  return (await encode(flat, "image/jpeg", PICTURE_QUALITY)) ?? png;
}

/** Shrinks an oversized GIF in a worker (inline where workers are unavailable, as in tests). */
function shrinkGifOffPage(bytes: Uint8Array): Promise<{ bytes: Uint8Array; edge: number } | undefined> {
  const inline = () => {
    try {
      return shrinkGif(bytes);
    } catch (error) {
      if (error instanceof GifShrinkError) return undefined;
      throw error;
    }
  };
  if (typeof Worker === "undefined") return Promise.resolve(inline());
  return new Promise((resolve) => {
    let worker: Worker;
    try {
      worker = new Worker(new URL("./gif-shrink-worker.ts", import.meta.url), { type: "module", name: "caper-gif" });
    } catch {
      resolve(inline());
      return;
    }
    const done = (value?: { bytes: Uint8Array; edge: number }) => {
      worker.terminate();
      resolve(value);
    };
    worker.onmessage = (event: MessageEvent<{ bytes?: Uint8Array; edge?: number }>) =>
      done(event.data.bytes && event.data.edge ? { bytes: event.data.bytes, edge: event.data.edge } : undefined);
    worker.onerror = () => done();
    worker.postMessage({ bytes }, [bytes.buffer]);
  });
}

/**
 * GIFs (by their magic bytes) keep animating: within 1024 × 1024 they upload
 * unchanged at any byte size, and a wider or taller one is shrunk to a 512 px
 * square, every frame kept. Anything else the browser decodes is center-cropped square,
 * scaled to at most 512 px and re-encoded as WebP, which also drops metadata
 * such as location.
 */
export async function preparePicture(file: Blob): Promise<PreparedPicture> {
  const gif = gifSize(new Uint8Array(await file.slice(0, 10).arrayBuffer()));
  if (gif) {
    if (gifUploadsUnchanged(gif)) return { blob: file, contentType: "image/gif", ...gif };
    const shrunk = await shrinkGifOffPage(new Uint8Array(await file.arrayBuffer()));
    if (!shrunk) throw new PictureError(pictureMessages.gifUnreadable);
    const blob = new Blob([shrunk.bytes as BlobPart], { type: "image/gif" });
    return { blob, contentType: "image/gif", width: shrunk.edge, height: shrunk.edge };
  }
  const image = await decode(file);
  try {
    const crop = squareCrop(image.width, image.height);
    const edge = pictureEdge(image.width, image.height);
    const canvas = surface(edge, edge);
    const context = canvas.getContext("2d") as Context2D | null;
    if (!context) throw new PictureError(pictureMessages.undecodable);
    context.imageSmoothingQuality = "high";
    context.drawImage(image.source, crop.x, crop.y, crop.side, crop.side, 0, 0, edge, edge);
    const blob = (await encode(canvas, "image/webp", PICTURE_QUALITY)) ?? (await fallback(canvas));
    if (!blob) throw new PictureError(pictureMessages.undecodable);
    return { blob, contentType: blob.type, width: edge, height: edge };
  } finally {
    image.close();
  }
}

export type PictureTarget = { purpose: "avatar" } | { purpose: "spaceIcon"; spaceId: string };

async function failure(response: Response, purpose: PicturePurpose) {
  const body = (await response.json().catch(() => undefined)) as { error?: unknown; code?: unknown } | undefined;
  return new PictureError(pictureErrorMessage(response.status, body, purpose), response.status);
}

/** Reserve, PUT with exactly the signed headers, then complete (which applies it). Returns the completion body. */
export async function uploadPicture(
  picture: PreparedPicture,
  target: PictureTarget,
  progress: (fraction: number) => void,
  transport: UploadTransport = browserTransport,
  signal: AbortSignal = new AbortController().signal,
): Promise<Record<string, unknown>> {
  const created = await transport.fetch("/api/assets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    signal: AbortSignal.timeout(15_000),
    body: JSON.stringify({
      purpose: target.purpose,
      ...(target.purpose === "spaceIcon" ? { spaceId: target.spaceId } : {}),
      filename: pictureFilename(target.purpose, picture.contentType),
      contentType: picture.contentType,
      byteSize: picture.blob.size,
      width: picture.width,
      height: picture.height,
    }),
  });
  if (!created.ok) throw await failure(created, target.purpose);
  const reservation = (await created.json().catch(() => undefined)) as
    | { id?: unknown; upload?: { url?: unknown; headers?: Record<string, string> } }
    | undefined;
  if (typeof reservation?.id !== "string") throw new PictureError(pictureMessages.failed);
  const id = reservation.id;
  try {
    if (typeof reservation.upload?.url !== "string") throw new PictureError(pictureMessages.failed);
    await transport.put(reservation.upload.url, reservation.upload.headers ?? {}, picture.blob, progress, signal);
    const path = `/api/assets/${encodeURIComponent(id)}/complete`;
    for (let attempt = 0; ; attempt++) {
      const completed = await transport.fetch(path, { method: "POST", credentials: "same-origin", signal });
      if (completed.ok) {
        const body = (await completed.json().catch(() => undefined)) as Record<string, unknown> | undefined;
        if (!body || typeof body !== "object") throw new PictureError(pictureMessages.failed);
        progress(1);
        return body;
      }
      // 409: storage has not made the object visible yet. Anything else is final.
      if (completed.status !== 409 || attempt >= COMPLETE_RETRY_DELAYS_MS.length)
        throw await failure(completed, target.purpose);
      await completed.body?.cancel().catch(() => undefined);
      await (transport.sleep ?? sleep)(COMPLETE_RETRY_DELAYS_MS[attempt], signal);
    }
  } catch (error) {
    await cancelUpload(id, transport.fetch);
    throw error;
  }
}

/** Uploads a new profile picture and updates the account everywhere in this client. */
export async function uploadAvatar(file: Blob, progress: (fraction: number) => void, transport?: UploadTransport) {
  const body = await uploadPicture(await preparePicture(file), { purpose: "avatar" }, progress, transport);
  const account = body.account as Account | undefined;
  if (!account || typeof account.id !== "string") throw new PictureError(pictureMessages.failed);
  return rememberSavedAccount(account);
}

/** Uploads a new icon for a space you own. Returns the space as the server now has it. */
export async function uploadSpaceIcon(
  spaceId: string,
  file: Blob,
  progress: (fraction: number) => void,
  transport?: UploadTransport,
) {
  const body = await uploadPicture(await preparePicture(file), { purpose: "spaceIcon", spaceId }, progress, transport);
  const space = body.space as Space | undefined;
  if (!space || typeof space.id !== "string" || typeof space.name !== "string")
    throw new PictureError(pictureMessages.failed);
  return space;
}
