// Lossless metadata stripping for files uploaded as originals (nothing was
// re-encoded, so nothing else dropped their EXIF/GPS). Pixels and media
// samples are never touched; on anything unexpected the input is returned.

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset + text.length <= bytes.length && [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));
const text = (value: string) => new Uint8Array([...value].map((char) => char.charCodeAt(0)));

// ---- MP4 / QuickTime ----------------------------------------------------------

interface Box {
  type: string;
  start: number;
  header: number;
  end: number;
}

async function readBox(file: Blob, start: number, limit: number): Promise<Box | undefined> {
  if (start + 8 > limit) return;
  const head = new Uint8Array(await file.slice(start, Math.min(start + 16, limit)).arrayBuffer());
  const view = new DataView(head.buffer);
  const size32 = view.getUint32(0);
  const type = String.fromCharCode(...head.subarray(4, 8));
  let size = size32,
    header = 8;
  if (size32 === 1) {
    if (head.length < 16) return;
    size = Number(view.getBigUint64(8));
    header = 16;
  } else if (size32 === 0) {
    size = limit - start;
  }
  if (size < header || start + size > limit) return;
  return { type, start, header, end: start + size };
}

async function children(file: Blob, parent: { start: number; header: number; end: number }) {
  const boxes: Box[] = [];
  for (let offset = parent.start + parent.header; offset < parent.end;) {
    const box = await readBox(file, offset, parent.end);
    if (!box) return; // malformed: leave the file alone
    boxes.push(box);
    offset = box.end;
  }
  return boxes;
}

const METADATA_BOXES = new Set(["udta", "meta"]);
/** A metadata box is replaced by zeros in memory; beyond this, keep the file. */
const MAX_METADATA_BOX = 64 * 1024 * 1024;

/** Neutralizes every `udta` and `meta` box directly under `moov` and under
 * each `moov/trak` (iPhone `com.apple.quicktime.location.ISO6709`, Android
 * `©xyz`, titles, device names): the type becomes `free` and the contents
 * zeros of the same length, so no offset moves and sample data is untouched.
 * Reads only box headers through `slice`, never the whole file. */
export async function stripMp4Metadata(file: Blob): Promise<Blob> {
  try {
    const top = await children(file, { start: 0, header: 0, end: file.size });
    const moov = top?.find((box) => box.type === "moov");
    if (!top?.length || !moov || !["ftyp", "moov", "wide", "free", "mdat", "skip"].includes(top[0].type)) return file;
    const level = await children(file, moov);
    if (!level) return file;
    const targets = level.filter((box) => METADATA_BOXES.has(box.type));
    for (const trak of level.filter((box) => box.type === "trak")) {
      const inner = await children(file, trak);
      if (!inner) return file;
      targets.push(...inner.filter((box) => METADATA_BOXES.has(box.type)));
    }
    if (!targets.length || targets.some((box) => box.end - box.start > MAX_METADATA_BOX)) return file;
    targets.sort((a, b) => a.start - b.start);
    const parts: BlobPart[] = [];
    let offset = 0;
    for (const box of targets) {
      parts.push(file.slice(offset, box.start));
      const replacement = new Uint8Array(box.end - box.start);
      replacement.set(new Uint8Array(await file.slice(box.start, box.start + box.header).arrayBuffer()));
      replacement.set(text("free"), 4);
      parts.push(replacement);
      offset = box.end;
    }
    parts.push(file.slice(offset));
    return new Blob(parts, { type: file.type });
  } catch {
    return file;
  }
}

// ---- JPEG -----------------------------------------------------------------------

/** EXIF Orientation (1–8) from an APP1 Exif payload, if present. */
export function exifOrientation(app1: Uint8Array): number | undefined {
  if (!ascii(app1, 0, "Exif\0\0") || app1.length < 14) return;
  const tiff = app1.subarray(6);
  const view = new DataView(tiff.buffer, tiff.byteOffset, tiff.byteLength);
  const little = ascii(tiff, 0, "II");
  if (!little && !ascii(tiff, 0, "MM")) return;
  const ifd = view.getUint32(4, little);
  if (ifd + 2 > tiff.length) return;
  const count = view.getUint16(ifd, little);
  for (let i = 0; i < count; i++) {
    const entry = ifd + 2 + i * 12;
    if (entry + 12 > tiff.length) return;
    if (view.getUint16(entry, little) === 0x0112) {
      const value = view.getUint16(entry + 8, little);
      return value >= 1 && value <= 8 ? value : undefined;
    }
  }
}

/** A minimal APP1 segment holding only the Orientation tag. */
export function orientationSegment(orientation: number) {
  const payload = new Uint8Array([
    ...text("Exif\0\0"),
    0x4d,
    0x4d,
    0x00,
    0x2a,
    0x00,
    0x00,
    0x00,
    0x08, // big-endian TIFF, IFD0 at 8
    0x00,
    0x01, // one entry
    0x01,
    0x12,
    0x00,
    0x03,
    0x00,
    0x00,
    0x00,
    0x01,
    0x00,
    orientation,
    0x00,
    0x00, // Orientation SHORT
    0x00,
    0x00,
    0x00,
    0x00, // no next IFD
  ]);
  const length = payload.length + 2;
  return new Uint8Array([0xff, 0xe1, length >> 8, length & 0xff, ...payload]);
}

/** Keeps APP0 (JFIF), APP2 ICC_PROFILE and APP14 (Adobe colour transform);
 * drops APP1 (Exif, XMP), every other APPn and comments. A non-default EXIF
 * orientation survives as a minimal Exif segment. Scan data is copied as-is. */
export async function stripJpegMetadata(file: Blob): Promise<Blob> {
  try {
    // Metadata lives before the scan; 1 MiB covers huge XMP blocks, and a
    // header that does not fit is left alone.
    const head = new Uint8Array(await file.slice(0, 1024 * 1024).arrayBuffer());
    if (head[0] !== 0xff || head[1] !== 0xd8) return file;
    const kept: Uint8Array[] = [head.subarray(0, 2)];
    let orientation: number | undefined;
    let removed = false;
    let insertAt = 1;
    let offset = 2;
    for (;;) {
      if (offset + 4 > head.length || head[offset] !== 0xff) return file;
      const marker = head[offset + 1];
      if (marker === 0xff) {
        offset++;
        continue;
      } // fill byte
      if (marker === 0xda) break; // start of scan: the rest is image data
      if (marker === 0xd9 || (marker >= 0xd0 && marker <= 0xd7) || marker === 0x01) return file;
      const length = (head[offset + 2] << 8) | head[offset + 3];
      const end = offset + 2 + length;
      if (length < 2 || end > head.length) return file;
      const segment = head.subarray(offset, end);
      const payload = segment.subarray(4);
      const app = marker >= 0xe0 && marker <= 0xef;
      const keep =
        (!app && marker !== 0xfe) ||
        marker === 0xe0 ||
        marker === 0xee ||
        (marker === 0xe2 && ascii(payload, 0, "ICC_PROFILE\0"));
      if (marker === 0xe1) orientation ??= exifOrientation(payload);
      if (keep) {
        kept.push(segment);
        if (marker === 0xe0) insertAt = kept.length;
      } else {
        removed = true;
      }
      offset = end;
    }
    if (!removed) return file;
    if (orientation && orientation !== 1) kept.splice(insertAt, 0, orientationSegment(orientation));
    return new Blob([...(kept as BlobPart[]), file.slice(offset)], { type: file.type });
  } catch {
    return file;
  }
}

// ---- PNG ------------------------------------------------------------------------

const PNG_METADATA = new Set(["eXIf", "tEXt", "zTXt", "iTXt"]);

/** Drops eXIf and text chunks (XMP, EXIF, comments). Kept chunks, including
 * their CRCs, are copied byte for byte. */
export async function stripPngMetadata(file: Blob): Promise<Blob> {
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (bytes.length < 8 || bytes[0] !== 0x89 || !ascii(bytes, 1, "PNG\r\n\x1a\n")) return file;
    const view = new DataView(bytes.buffer);
    const parts: Uint8Array[] = [bytes.subarray(0, 8)];
    let removed = false;
    let offset = 8;
    while (offset < bytes.length) {
      if (offset + 12 > bytes.length) return file;
      const end = offset + 12 + view.getUint32(offset);
      if (end > bytes.length) return file;
      const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
      if (PNG_METADATA.has(type)) removed = true;
      else parts.push(bytes.subarray(offset, end));
      offset = end;
      if (type === "IEND") break;
    }
    return removed ? new Blob(parts as BlobPart[], { type: file.type }) : file;
  } catch {
    return file;
  }
}

/** Strips what the container allows for an original upload of `contentType`. */
export function stripMetadata(file: Blob, contentType: string): Promise<Blob> {
  switch (contentType) {
    case "image/jpeg":
      return stripJpegMetadata(file);
    case "image/png":
      return stripPngMetadata(file);
    case "video/mp4":
    case "video/quicktime":
    case "video/3gpp":
    case "audio/mp4":
    case "audio/x-m4a":
      return stripMp4Metadata(file);
    default:
      return Promise.resolve(file);
  }
}
