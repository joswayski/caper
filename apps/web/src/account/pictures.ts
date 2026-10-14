// Custom profile pictures and space icons: the pure rules every surface shares.
// Browser preparation and upload live in `picture-upload.ts`; rendering in
// `components/Picture.tsx`.

/** GIFs upload unchanged so they keep animating; the server accepts up to 5 MiB. */
export const MAX_GIF_BYTES = 5 * 1024 * 1024;
/** Every other picture is re-encoded and must end at or under 1 MiB. */
export const MAX_PICTURE_BYTES = 1024 * 1024;
/** The server's largest accepted width or height, in pixels. */
export const MAX_PICTURE_EDGE = 1024;
/** Re-encoded pictures are square, at most this many pixels on a side. */
export const PICTURE_EDGE = 512;

export type PicturePurpose = "avatar" | "spaceIcon";

/** A person's mascot plus, optionally, their custom picture. */
export interface PersonPicture {
  avatarId?: number | null;
  /** Stable ID of the current picture: the cache key, never the URL. */
  avatarAsset?: string;
  /** Signed CDN URL for `avatarAsset`; re-signed daily. */
  avatarUrl?: string;
}

export interface SpaceIcon {
  iconAsset?: string;
  iconUrl?: string;
}

export const pictureMessages = {
  gifLimit: "GIFs can be up to 5 MB and 1024 × 1024.",
  undecodable: "This browser can’t open that picture. Try a PNG, JPEG, WebP or GIF.",
  tooLarge: "That picture is too large. Try a smaller one.",
  type: "Use a PNG, JPEG, WebP or GIF picture.",
  dimensions: "Pictures can be up to 1024 × 1024.",
  serverTooLarge: "GIFs can be up to 5 MB, and other pictures up to 1 MB.",
  rateLimited: "You’ve changed pictures too many times today. Try again tomorrow.",
  invalid: "That picture couldn’t be used. Try another one.",
  unavailable: "Picture uploads aren’t available right now.",
  notOwner: "Only the space owner can change its icon.",
  signedOut: "You’re signed out. Sign in again to continue.",
  failed: "Your picture couldn’t be uploaded. Try again.",
} as const;

/** `GIF87a`/`GIF89a` and the logical screen size (bytes 6–9, little-endian), whatever the file is called. */
export function gifSize(bytes: Uint8Array): { width: number; height: number } | undefined {
  if (bytes.length < 10) return;
  const signature = String.fromCharCode(...bytes.subarray(0, 6));
  if (signature !== "GIF87a" && signature !== "GIF89a") return;
  return { width: bytes[6] | (bytes[7] << 8), height: bytes[8] | (bytes[9] << 8) };
}

/** Why a GIF can't be uploaded unchanged, if it can't. */
export function gifError(byteSize: number, size: { width: number; height: number }) {
  if (
    byteSize > MAX_GIF_BYTES ||
    size.width < 1 ||
    size.height < 1 ||
    size.width > MAX_PICTURE_EDGE ||
    size.height > MAX_PICTURE_EDGE
  )
    return pictureMessages.gifLimit;
}

/** The centered square of a `width` × `height` image, in source pixels. */
export function squareCrop(width: number, height: number) {
  const side = Math.max(1, Math.min(width, height));
  return { x: Math.floor((width - side) / 2), y: Math.floor((height - side) / 2), side };
}

/** Output edge of a re-encoded picture: its short edge, never enlarged, at most 512. */
export function pictureEdge(width: number, height: number) {
  return Math.max(1, Math.min(PICTURE_EDGE, Math.floor(width), Math.floor(height)));
}

/** `avatar.webp`, `space-icon.gif`… */
export function pictureFilename(purpose: PicturePurpose, contentType: string) {
  const extension =
    ({ "image/webp": "webp", "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif" } as Record<string, string>)[
      contentType
    ] ?? "bin";
  return `${purpose === "avatar" ? "avatar" : "space-icon"}.${extension}`;
}

/** Readable text for a failed picture request (`{error, code?}` from the API). */
export function pictureErrorMessage(
  status: number,
  body: { error?: unknown; code?: unknown } | undefined,
  purpose: PicturePurpose,
) {
  const code = typeof body?.code === "string" ? body.code : undefined;
  if (code === "picture_type") return pictureMessages.type;
  if (code === "picture_too_large" || status === 413) return pictureMessages.serverTooLarge;
  if (code === "picture_rate_limited" || status === 429) return pictureMessages.rateLimited;
  if (code === "picture_invalid" || status === 422) return pictureMessages.invalid;
  if (status === 503) return pictureMessages.unavailable;
  if (status === 401) return pictureMessages.signedOut;
  if (status === 404) return purpose === "spaceIcon" ? pictureMessages.notOwner : pictureMessages.unavailable;
  if (status === 400)
    return typeof body?.error === "string" && /width|height|dimension|pixel/i.test(body.error)
      ? pictureMessages.dimensions
      : pictureMessages.invalid;
  return pictureMessages.failed;
}

/** `person` with the account's picture: a saved change wins over older snapshots. */
export function withPicture<T extends object>(person: T, picture: PersonPicture): T {
  const held = person as T & PersonPicture;
  const asset = picture.avatarAsset || undefined;
  // Unchanged people keep their identity, so memoized rows don't re-render.
  if (
    (picture.avatarId === undefined || held.avatarId === picture.avatarId) &&
    held.avatarAsset === asset &&
    held.avatarUrl === (asset && picture.avatarUrl ? picture.avatarUrl : undefined)
  )
    return person;
  const { avatarAsset: _asset, avatarUrl: _url, ...rest } = held;
  return {
    ...rest,
    ...(picture.avatarId !== undefined ? { avatarId: picture.avatarId } : {}),
    ...(picture.avatarAsset ? { avatarAsset: picture.avatarAsset } : {}),
    ...(picture.avatarAsset && picture.avatarUrl ? { avatarUrl: picture.avatarUrl } : {}),
  } as T;
}

/** A space after an icon change: the server's copy, keeping only what it doesn't send (an icon it omits is gone). */
export function withSpaceIcon<T extends SpaceIcon>(space: T, server: Partial<T> & SpaceIcon): T {
  const { iconAsset: _asset, iconUrl: _url, ...rest } = space;
  return { ...rest, ...server } as T;
}

// ---- Display cache -----------------------------------------------------------
// Keyed by asset ID, so a re-signed URL for a picture already shown reuses the
// loaded copy instead of downloading it again. Bounded for long member lists.

const MAX_REMEMBERED = 512;
/** asset -> a URL that loaded. */
const loaded = new Map<string, string>();
/** asset -> the latest URL that failed; a different URL for it is tried again. */
const failed = new Map<string, string>();

function remember(map: Map<string, string>, asset: string, url: string) {
  map.delete(asset);
  map.set(asset, url);
  if (map.size > MAX_REMEMBERED) map.delete(map.keys().next().value!);
}

/** Only absolute http(s) URLs from the API become image sources. */
export function pictureUrl(url: unknown): string | undefined {
  return typeof url === "string" && /^https?:\/\//i.test(url) ? url : undefined;
}

/** The URL to show for `asset`, or undefined for the mascot/initial (none, or it already failed). */
export function pictureSource(asset: string | undefined, url: string | undefined): string | undefined {
  const fresh = pictureUrl(url);
  if (!asset || !fresh) return;
  const known = loaded.get(asset);
  if (known) return known;
  return failed.get(asset) === fresh ? undefined : fresh;
}

/** True when `url` already loaded on this page, so it can appear without fading in. */
export function pictureShown(asset: string, url: string) {
  return loaded.get(asset) === url;
}

export function pictureLoaded(asset: string, url: string) {
  failed.delete(asset);
  remember(loaded, asset, url);
}

export function pictureFailed(asset: string, url: string) {
  if (loaded.get(asset) === url) loaded.delete(asset);
  remember(failed, asset, url);
}

/** For tests: forget every picture. */
export function resetPicturesForTests() {
  loaded.clear();
  failed.clear();
}
