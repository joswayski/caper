// Removes identifying metadata (location, serial numbers, names, dates, tags)
// from every upload before it leaves the browser. The server does not clean
// uploads, so a format this cannot clean is refused rather than sent.
//
// The rules are shared with the native clients (`shared/media/metadata.rs`) and
// pinned by `shared/media/metadata/cases.json`: every implementation produces
// byte-identical output. Pixels and media samples are never re-encoded. Large
// files are read in slices; only edited byte ranges are held in memory.

export type Refusal = "unsupported" | "malformed";

/** A file whose hidden details can't be removed, so it must not be uploaded. */
export class MetadataRefused extends Error {
  constructor(readonly reason: Refusal) {
    super(`metadata ${reason}`);
    this.name = "MetadataRefused";
  }
}

/** Replace `len` source bytes at `offset` with `bytes`, or with zeros. */
export interface Edit {
  offset: number;
  len: number;
  bytes?: Uint8Array;
}

const malformed = () => new MetadataRefused("malformed");
const unsupported = () => new MetadataRefused("unsupported");

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset >= 0 &&
  offset + text.length <= bytes.length &&
  [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));
const text = (value: string) => Uint8Array.from(value, (char) => char.charCodeAt(0));
const fourcc = (bytes: Uint8Array, offset: number) => String.fromCharCode(...bytes.subarray(offset, offset + 4));
const be16 = (b: Uint8Array, o: number) => (b[o] << 8) | b[o + 1];
const be32 = (b: Uint8Array, o: number) => ((b[o] << 24) | (b[o + 1] << 16) | (b[o + 2] << 8) | b[o + 3]) >>> 0;
const le16 = (b: Uint8Array, o: number) => b[o] | (b[o + 1] << 8);
const le32 = (b: Uint8Array, o: number) => (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
const be64 = (b: Uint8Array, o: number) => be32(b, o) * 2 ** 32 + be32(b, o + 4);
const uint = (b: Uint8Array, o: number, size: number, little: boolean) => {
  let value = 0;
  for (let i = 0; i < size; i++) value = value * 256 + b[little ? o + size - 1 - i : o + i];
  return value;
};
const concat = (parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
};

const CHUNK = 1024 * 1024;

/** Random access to a Blob with a one-chunk read cache. Reads outside the file
 * are malformed input. */
class Source {
  readonly size: number;
  private cacheStart = -1;
  private cache = new Uint8Array();
  constructor(private readonly blob: Blob) {
    this.size = blob.size;
  }

  async read(offset: number, len: number): Promise<Uint8Array> {
    if (offset < 0 || len < 0 || offset + len > this.size) throw malformed();
    if (this.cacheStart >= 0 && offset >= this.cacheStart && offset + len <= this.cacheStart + this.cache.length)
      return this.cache.subarray(offset - this.cacheStart, offset - this.cacheStart + len);
    if (len > CHUNK) return new Uint8Array(await this.blob.slice(offset, offset + len).arrayBuffer());
    this.cacheStart = offset;
    this.cache = new Uint8Array(await this.blob.slice(offset, Math.min(this.size, offset + CHUNK)).arrayBuffer());
    return this.cache.subarray(0, len);
  }

  /** Up to `len` bytes, fewer at the end of the file. */
  async peek(offset: number, len: number) {
    return this.read(offset, Math.max(0, Math.min(len, this.size - offset)));
  }
}

class Edits {
  readonly list: Edit[] = [];
  zero(offset: number, len: number) {
    if (len > 0) this.list.push({ offset, len });
  }
  replace(offset: number, len: number, bytes: Uint8Array) {
    this.list.push({ offset, len, bytes });
  }
}

/** Sorted, with overlapping or adjacent zero ranges merged. */
function normalize(edits: Edit[], size: number): Edit[] {
  const sorted = [...edits].sort((a, b) => a.offset - b.offset || a.len - b.len);
  const out: Edit[] = [];
  for (const edit of sorted) {
    if (edit.offset < 0 || edit.offset + edit.len > size) throw malformed();
    const last = out.at(-1);
    if (last && edit.offset < last.offset + last.len) {
      if (last.bytes || edit.bytes) throw malformed();
      last.len = Math.max(last.len, edit.offset + edit.len - last.offset);
    } else if (last && !last.bytes && !edit.bytes && edit.offset === last.offset + last.len) {
      last.len += edit.len;
    } else {
      out.push({ ...edit });
    }
  }
  return out;
}

/** The size after applying `edits`. */
export function cleanedLength(size: number, edits: Edit[]) {
  return edits.reduce((total, edit) => total + (edit.bytes ? edit.bytes.length - edit.len : 0), size);
}

/** `file` with `edits` applied. Zero runs are materialized in bounded pieces. */
export function applyEdits(file: Blob, edits: Edit[], type = file.type): Blob {
  if (!edits.length) return file;
  const parts: BlobPart[] = [];
  let offset = 0;
  for (const edit of edits) {
    parts.push(file.slice(offset, edit.offset));
    if (edit.bytes) parts.push(edit.bytes as BlobPart);
    else for (let left = edit.len; left > 0; left -= CHUNK) parts.push(new Uint8Array(Math.min(left, CHUNK)));
    offset = edit.offset + edit.len;
  }
  parts.push(file.slice(offset));
  return new Blob(parts, { type });
}

// ---- Classification ---------------------------------------------------------------

export type Format =
  | "jpeg"
  | "png"
  | "gif"
  | "webp"
  | "heif"
  | "mp4"
  | "matroska"
  | "flac"
  | "ogg"
  | "wav"
  | "avi"
  | "aiff"
  | "mpeg"
  | "tiff"
  | "raf"
  | "crw"
  | "jxl"
  | "jp2"
  | "psd"
  | "unchanged";

const HEIF_BRANDS = ["avif", "avis", "mif1", "msf1", "heic", "heix", "hevc", "hevx", "heim", "heis", "heif"];

/** What the first bytes are, never the name or declared type. Declared media
 * that matches no cleanable format is refused. */
export function classify(head: Uint8Array, declared: string): Format {
  const at = (offset: number, value: string | number[]) =>
    typeof value === "string" ? ascii(head, offset, value) : value.every((byte, i) => head[offset + i] === byte);
  if (at(0, [0xff, 0xd8, 0xff])) return "jpeg";
  if (at(0, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return "png";
  if (at(0, "GIF87a") || at(0, "GIF89a")) return "gif";
  if (at(0, "RIFF") && at(8, "WEBP")) return "webp";
  if (at(4, "ftyp")) {
    const brand = fourcc(head, 8);
    if (HEIF_BRANDS.includes(brand)) return "heif";
    if (brand === "crx ") throw unsupported();
    return "mp4";
  }
  if (["moov", "mdat", "wide", "free"].some((tag) => at(4, tag))) return "mp4";
  if (at(0, [0x1a, 0x45, 0xdf, 0xa3])) return "matroska";
  if (at(0, "fLaC")) return "flac";
  if (at(0, "OggS")) return "ogg";
  if (at(0, "RIFF") && at(8, "WAVE")) return "wav";
  if (at(0, "RIFF") && at(8, "AVI ")) return "avi";
  if (at(0, "FORM") && (at(8, "AIFF") || at(8, "AIFC"))) return "aiff";
  if (at(0, "ID3") || (head.length >= 2 && head[0] === 0xff && (head[1] & 0xe0) === 0xe0)) return "mpeg";
  if (at(0, [0xff, 0x0a])) return "unchanged";
  if (["II\x2a\0", "MM\0\x2a", "IIU\0", "IIRO", "IIRS", "MMOR"].some((magic) => at(0, magic))) return "tiff";
  if (at(0, "FUJIFILMCCD-RAW")) return "raf";
  if (at(0, "II\x1a\0\0\0HEAPCCDR")) return "crw";
  if (at(0, "\0\0\0\x0cJXL \r\n\x87\n")) return "jxl";
  if (at(0, "\0\0\0\x0cjP  ")) return "jp2";
  if (at(0, "8BPS")) return "psd";
  if (["image/", "video/", "audio/"].some((prefix) => declared.startsWith(prefix))) throw unsupported();
  return "unchanged";
}

/** The edits that clean `file`, or a refusal. */
export async function metadataEdits(file: Blob, declaredType: string): Promise<Edit[]> {
  const declared = declaredType.split(";")[0].trim().toLowerCase() || "application/octet-stream";
  const source = new Source(file);
  const format = classify(await source.peek(0, 64), declared);
  const edits = new Edits();
  switch (format) {
    case "jpeg":
      edits.replace(0, file.size, cleanJpeg(await source.read(0, file.size)));
      break;
    case "png":
      edits.replace(0, file.size, cleanPng(await source.read(0, file.size)));
      break;
    case "gif":
      edits.replace(0, file.size, cleanGif(await source.read(0, file.size)));
      break;
    case "webp":
      edits.replace(0, file.size, cleanWebp(await source.read(0, file.size)));
      break;
    case "heif":
      await cleanHeif(source, edits);
      break;
    case "mp4":
      await cleanMp4(source, edits);
      break;
    case "matroska":
      await cleanMatroska(source, edits);
      break;
    case "flac":
      await cleanTrailingTags(source, edits, await cleanFlac(source, edits, 0));
      break;
    case "mpeg":
      await cleanMpeg(source, edits);
      break;
    case "ogg":
      await cleanOgg(source, edits);
      break;
    case "wav":
      await cleanRiffAudio(source, edits, false);
      break;
    case "aiff":
      await cleanRiffAudio(source, edits, true);
      break;
    case "avi":
      await cleanAvi(source, edits);
      break;
    case "tiff":
      await cleanTiff(source, edits, 0, file.size);
      break;
    case "raf":
      await cleanRaf(source, edits);
      break;
    case "crw":
      await cleanCrw(source, edits);
      break;
    case "psd":
      await cleanPsd(source, edits);
      break;
    case "jxl":
    case "jp2":
      await deleteTopBoxes(source, edits, format);
      break;
    case "unchanged":
      break;
  }
  const list = normalize(edits.list, file.size);
  // A whole-file rewrite that changed nothing is no edit at all.
  if (list.length === 1 && list[0].bytes && list[0].offset === 0 && list[0].len === file.size) {
    const original = await source.read(0, file.size);
    const bytes = list[0].bytes;
    if (bytes.length === original.length && bytes.every((byte, i) => byte === original[i])) return [];
  }
  return list;
}

/** `file` without its identifying metadata. Throws `MetadataRefused`. */
export async function cleanMetadata(file: Blob, declaredType: string): Promise<Blob> {
  return applyEdits(file, await metadataEdits(file, declaredType), file.type);
}

// ---- Orientation --------------------------------------------------------------------

/** EXIF Orientation (1–8) from a TIFF block, optionally after `Exif\0\0`. */
export function tiffOrientation(block: Uint8Array): number | undefined {
  const tiff = ascii(block, 0, "Exif\0\0") ? block.subarray(6) : block;
  const little = ascii(tiff, 0, "II");
  if (!little && !ascii(tiff, 0, "MM")) return;
  if (tiff.length < 8) return;
  const u16 = (o: number) => (little ? le16(tiff, o) : be16(tiff, o));
  const ifd = little ? le32(tiff, 4) : be32(tiff, 4);
  if (ifd + 2 > tiff.length) return;
  const count = u16(ifd);
  for (let i = 0; i < count; i++) {
    const entry = ifd + 2 + i * 12;
    if (entry + 12 > tiff.length) return;
    if (u16(entry) === 0x0112) {
      const value = u16(entry + 8);
      return value >= 1 && value <= 8 ? value : undefined;
    }
  }
}

/** A big-endian TIFF holding only the Orientation tag (26 bytes). */
const orientationTiff = (orientation: number) =>
  Uint8Array.from([
    0x4d,
    0x4d,
    0,
    0x2a,
    0,
    0,
    0,
    8,
    0,
    1,
    0x01,
    0x12,
    0,
    3,
    0,
    0,
    0,
    1,
    0,
    orientation,
    0,
    0,
    0,
    0,
    0,
    0,
  ]);

const keepsOrientation = (orientation: number | undefined): orientation is number =>
  orientation !== undefined && orientation >= 2 && orientation <= 8;

// ---- JPEG -----------------------------------------------------------------------------

const keptJpegSegment = (marker: number, payload: Uint8Array) => {
  const app = marker >= 0xe0 && marker <= 0xef;
  return (
    (!app && marker !== 0xfe) ||
    marker === 0xe0 ||
    marker === 0xee ||
    (marker === 0xe2 && ascii(payload, 0, "ICC_PROFILE\0"))
  );
};

function cleanJpeg(b: Uint8Array): Uint8Array {
  const out: Uint8Array[] = [b.subarray(0, 2)];
  let orientation: number | undefined;
  let insertAt = 1;
  let p = 2;
  for (;;) {
    while (p + 1 < b.length && b[p] === 0xff && b[p + 1] === 0xff) p++;
    if (p + 4 > b.length || b[p] !== 0xff) throw malformed();
    const marker = b[p + 1];
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd9)) throw malformed();
    const end = p + 2 + be16(b, p + 2);
    if (be16(b, p + 2) < 2 || end > b.length) throw malformed();
    const payload = b.subarray(p + 4, end);
    if (marker === 0xe1 && orientation === undefined && ascii(payload, 0, "Exif\0\0"))
      orientation = tiffOrientation(payload) ?? 0;
    if (keptJpegSegment(marker, payload)) {
      out.push(b.subarray(p, end));
      if (marker === 0xe0) insertAt = out.length;
    }
    p = end;
    if (marker === 0xda) break;
  }
  // Entropy-coded data, then markers between scans, until EOI.
  let start = p;
  for (;;) {
    let q = p;
    while (q + 1 < b.length && !(b[q] === 0xff && b[q + 1] !== 0 && b[q + 1] !== 0xff && (b[q + 1] & 0xf8) !== 0xd0))
      q++;
    if (q + 1 >= b.length) {
      out.push(b.subarray(start));
      break;
    }
    out.push(b.subarray(start, q));
    const marker = b[q + 1];
    if (marker === 0xd9) {
      out.push(b.subarray(q, q + 2));
      break;
    }
    if (q + 4 > b.length) throw malformed();
    const end = q + 2 + be16(b, q + 2);
    if (be16(b, q + 2) < 2 || end > b.length) throw malformed();
    if (keptJpegSegment(marker, b.subarray(q + 4, end))) out.push(b.subarray(q, end));
    start = p = end;
  }
  if (keepsOrientation(orientation))
    out.splice(
      insertAt,
      0,
      concat([Uint8Array.from([0xff, 0xe1, 0, 0x22]), text("Exif\0\0"), orientationTiff(orientation)]),
    );
  return concat(out);
}

// ---- PNG -------------------------------------------------------------------------------

const PNG_DROPPED = new Set(["tEXt", "zTXt", "iTXt", "tIME", "caBX"]);

let crcTable: Uint32Array | undefined;
function pngCrc(bytes: Uint8Array) {
  crcTable ??= Uint32Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  let crc = 0xffffffff;
  for (const byte of bytes) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function cleanPng(b: Uint8Array): Uint8Array {
  const out: Uint8Array[] = [b.subarray(0, 8)];
  let exifSeen = false;
  let p = 8;
  for (;;) {
    if (p + 12 > b.length) throw malformed();
    const end = p + 12 + be32(b, p);
    if (end > b.length) throw malformed();
    const type = fourcc(b, p + 4);
    if (type === "eXIf") {
      const orientation = exifSeen ? undefined : tiffOrientation(b.subarray(p + 8, end - 4));
      exifSeen = true;
      if (keepsOrientation(orientation)) {
        const body = concat([text("eXIf"), orientationTiff(orientation)]);
        const crc = pngCrc(body);
        out.push(
          concat([
            Uint8Array.from([0, 0, 0, 26]),
            body,
            Uint8Array.from([crc >>> 24, crc >> 16, crc >> 8, crc].map((v) => v & 0xff)),
          ]),
        );
      }
    } else if (!PNG_DROPPED.has(type)) {
      out.push(b.subarray(p, end));
    }
    p = end;
    if (type === "IEND") break;
  }
  return concat(out);
}

// ---- GIF ---------------------------------------------------------------------------------

const GIF_APPLICATIONS = ["NETSCAPE2.0", "ANIMEXTS1.0", "ICCRGBG1012"];

function cleanGif(b: Uint8Array): Uint8Array {
  if (b.length < 13) throw malformed();
  let p = 13 + (b[10] & 0x80 ? 3 * 2 ** ((b[10] & 7) + 1) : 0);
  if (p > b.length) throw malformed();
  const out: Uint8Array[] = [b.subarray(0, p)];
  const subBlocks = (from: number) => {
    let q = from;
    for (;;) {
      if (q >= b.length) throw malformed();
      if (b[q] === 0) return q + 1;
      q += 1 + b[q];
    }
  };
  for (;;) {
    if (p >= b.length) throw malformed();
    const kind = b[p];
    if (kind === 0x3b) {
      out.push(b.subarray(p, p + 1));
      break;
    }
    if (kind === 0x2c) {
      if (p + 10 > b.length) throw malformed();
      const flags = b[p + 9];
      const data = p + 10 + (flags & 0x80 ? 3 * 2 ** ((flags & 7) + 1) : 0) + 1;
      const end = subBlocks(data);
      out.push(b.subarray(p, end));
      p = end;
    } else if (kind === 0x21) {
      if (p + 2 > b.length) throw malformed();
      const label = b[p + 1];
      const end = subBlocks(p + 2);
      const keep =
        label === 0xf9 ||
        label === 0x01 ||
        (label === 0xff && b[p + 2] === 11 && GIF_APPLICATIONS.some((name) => ascii(b, p + 3, name)));
      if (keep) out.push(b.subarray(p, end));
      p = end;
    } else {
      throw malformed();
    }
  }
  return concat(out);
}

// ---- WebP --------------------------------------------------------------------------------

const WEBP_KEPT = new Set(["VP8X", "VP8 ", "VP8L", "ALPH", "ANIM", "ANMF", "ICCP"]);

function cleanWebp(b: Uint8Array): Uint8Array {
  const end = 8 + le32(b, 4);
  if (end > b.length || end < 12) throw malformed();
  const out: Uint8Array[] = [new Uint8Array(12)];
  out[0].set(b.subarray(0, 12));
  let vp8x: Uint8Array | undefined;
  let exif = false;
  for (let p = 12; p < end;) {
    if (p + 8 > end) throw malformed();
    const size = le32(b, p + 4);
    const next = p + 8 + size + (size & 1);
    if (next > end) throw malformed();
    const type = fourcc(b, p);
    if (WEBP_KEPT.has(type)) {
      const chunk = b.slice(p, next);
      if (type === "VP8X" && !vp8x) vp8x = chunk;
      out.push(chunk);
    } else if (type === "EXIF") {
      const orientation = tiffOrientation(b.subarray(p + 8, p + 8 + size));
      if (keepsOrientation(orientation)) {
        exif = true;
        out.push(concat([text("EXIF"), Uint8Array.from([26, 0, 0, 0]), orientationTiff(orientation)]));
      }
    }
    p = next;
  }
  if (vp8x && vp8x.length > 8) vp8x[8] &= exif ? ~0x04 : ~0x0c;
  const result = concat(out);
  const size = result.length - 8;
  result.set([size & 0xff, (size >> 8) & 0xff, (size >> 16) & 0xff, (size >>> 24) & 0xff], 4);
  return result;
}

// ---- ISO base media boxes ---------------------------------------------------------------

interface Box {
  type: string;
  start: number;
  header: number;
  end: number;
}

/** Boxes in `start..end`. At the top level, a trailing `mdat`/`free`/`skip`
 * may run past the end of the file, and fewer than 8 trailing bytes are
 * ignored. */
async function boxes(source: Source, start: number, end: number, top = false): Promise<Box[]> {
  const list: Box[] = [];
  for (let offset = start; offset < end;) {
    if (offset + 8 > end) {
      if (top) break;
      throw malformed();
    }
    const head = await source.read(offset, Math.min(16, end - offset));
    let size = be32(head, 0);
    let header = 8;
    const type = fourcc(head, 4);
    if (size === 1) {
      if (head.length < 16) throw malformed();
      size = be64(head, 8);
      header = 16;
    } else if (size === 0) {
      size = end - offset;
    }
    if (size < header) throw malformed();
    if (offset + size > end) {
      if (top && ["mdat", "free", "skip"].includes(type)) size = end - offset;
      else throw malformed();
    }
    list.push({ type, start: offset, header, end: offset + size });
    offset += size;
  }
  return list;
}

const childrenOf = (source: Source, box: Box, skip = 0) => boxes(source, box.start + box.header + skip, box.end);
const payloadOf = (source: Source, box: Box) => source.read(box.start + box.header, box.end - box.start - box.header);

function neutralizeBox(edits: Edits, box: Box) {
  edits.replace(box.start + 4, 4, text("free"));
  edits.zero(box.start + box.header, box.end - box.start - box.header);
}

const METADATA_BOXES = ["udta", "meta", "uuid"];

/** `udta`, `meta` and `uuid` directly under `moov` and each `moov/trak`. */
async function cleanMoov(source: Source, edits: Edits, moov: Box, dropTracks: boolean) {
  const dropped = new Set<number>();
  let fragmented = false;
  for (const child of await childrenOf(source, moov)) {
    if (METADATA_BOXES.includes(child.type)) neutralizeBox(edits, child);
    else if (child.type === "mvex") fragmented = true;
    else if (child.type === "trak") {
      const inner = await childrenOf(source, child);
      if (dropTracks) {
        const mdia = inner.find((box) => box.type === "mdia");
        const hdlr = mdia && (await childrenOf(source, mdia)).find((box) => box.type === "hdlr");
        if (!mdia || !hdlr) throw malformed();
        const handler = fourcc(await payloadOf(source, hdlr), 8);
        if (handler !== "vide" && handler !== "soun") {
          await zeroTrackSamples(source, edits, mdia);
          const tkhd = inner.find((box) => box.type === "tkhd");
          if (tkhd) {
            const payload = await payloadOf(source, tkhd);
            dropped.add(be32(payload, payload[0] === 1 ? 20 : 12));
          }
          neutralizeBox(edits, child);
          continue;
        }
      }
      for (const box of inner) if (METADATA_BOXES.includes(box.type)) neutralizeBox(edits, box);
    }
  }
  return fragmented ? dropped : new Set<number>();
}

async function zeroTrackSamples(source: Source, edits: Edits, mdia: Box) {
  const minf = (await childrenOf(source, mdia)).find((box) => box.type === "minf");
  const stbl = minf && (await childrenOf(source, minf)).find((box) => box.type === "stbl");
  if (!stbl) throw malformed();
  const tables = await childrenOf(source, stbl);
  const stsz = tables.find((box) => box.type === "stsz" || box.type === "stz2");
  const stsc = tables.find((box) => box.type === "stsc");
  const stco = tables.find((box) => box.type === "stco" || box.type === "co64");
  if (!stsz || !stsc || !stco) throw malformed();
  const sz = await payloadOf(source, stsz);
  const sc = await payloadOf(source, stsc);
  const co = await payloadOf(source, stco);
  // stz2 packs sizes into 4, 8 or 16 bits; stsz has a fixed size or 32 bits.
  const field = stsz.type === "stz2" ? sz[7] : 32;
  if (![4, 8, 16, 32].includes(field)) throw malformed();
  const fixed = stsz.type === "stz2" ? 0 : be32(sz, 4);
  const sampleCount = be32(sz, 8);
  const sampleSize = (i: number) => {
    if (fixed) return fixed;
    const at = 12 + Math.floor((i * field) / 8);
    if (at + Math.max(1, field / 8) > sz.length) throw malformed();
    if (field === 4) return i % 2 ? sz[at] & 15 : sz[at] >> 4;
    return uint(sz, at, field / 8, false);
  };
  const wide = stco.type === "co64";
  const chunkCount = be32(co, 4);
  const runs = be32(sc, 4);
  if (8 + runs * 12 > sc.length || 8 + chunkCount * (wide ? 8 : 4) > co.length) throw malformed();
  let sample = 0;
  for (let chunk = 1; chunk <= chunkCount; chunk++) {
    let perChunk = 0;
    for (let r = 0; r < runs && be32(sc, 8 + r * 12) <= chunk; r++) perChunk = be32(sc, 8 + r * 12 + 4);
    const offset = wide ? be64(co, 8 + (chunk - 1) * 8) : be32(co, 8 + (chunk - 1) * 4);
    let length = 0;
    for (let i = 0; i < perChunk && sample < sampleCount; i++) length += sampleSize(sample++);
    if (offset + length > source.size) throw malformed();
    edits.zero(offset, length);
  }
}

async function cleanMp4(source: Source, edits: Edits) {
  const top = await boxes(source, 0, source.size, true);
  const moovs = top.filter((box) => box.type === "moov");
  if (moovs.length !== 1) throw malformed();
  for (const box of top) if (METADATA_BOXES.includes(box.type)) neutralizeBox(edits, box);
  const dropped = await cleanMoov(source, edits, moovs[0], true);
  if (!dropped.size) return;
  for (const moof of top.filter((box) => box.type === "moof"))
    for (const traf of (await childrenOf(source, moof)).filter((box) => box.type === "traf")) {
      const tfhd = (await childrenOf(source, traf)).find((box) => box.type === "tfhd");
      if (tfhd && dropped.has(be32(await payloadOf(source, tfhd), 4))) neutralizeBox(edits, traf);
    }
}

// ---- HEIF / AVIF -----------------------------------------------------------------------

async function cleanHeif(source: Source, edits: Edits) {
  const top = await boxes(source, 0, source.size, true);
  const meta = top.find((box) => box.type === "meta");
  if (meta) {
    const children = await childrenOf(source, meta, 4);
    const iinf = children.find((box) => box.type === "iinf");
    const iloc = children.find((box) => box.type === "iloc");
    const idat = children.find((box) => box.type === "idat");
    const cleaned = new Set<number>();
    if (iinf) {
      const version = (await source.read(iinf.start + iinf.header, 1))[0];
      for (const infe of await childrenOf(source, iinf, version === 0 ? 6 : 8)) {
        if (infe.type !== "infe") continue;
        const p = await payloadOf(source, infe);
        if (p[0] !== 2 && p[0] !== 3) continue;
        const id = p[0] === 2 ? be16(p, 4) : be32(p, 4);
        const typeAt = p[0] === 2 ? 8 : 10;
        const type = fourcc(p, typeAt);
        let q = typeAt + 4;
        while (q < p.length && p[q] !== 0) q++;
        let contentType = "";
        if (type === "mime") {
          const from = ++q;
          while (q < p.length && p[q] !== 0) q++;
          contentType = String.fromCharCode(...p.subarray(from, q));
        }
        if (type === "Exif" || (type === "mime" && contentType === "application/rdf+xml")) cleaned.add(id);
      }
    }
    if (cleaned.size && !iloc) throw malformed();
    if (cleaned.size && iloc) {
      const p = await payloadOf(source, iloc);
      const version = p[0];
      if (version > 2) throw malformed();
      const offsetSize = p[4] >> 4;
      const lengthSize = p[4] & 15;
      const baseSize = p[5] >> 4;
      const indexSize = version > 0 ? p[5] & 15 : 0;
      if ([offsetSize, lengthSize, baseSize, indexSize].some((size) => ![0, 4, 8].includes(size))) throw malformed();
      let q = 6;
      const take = (size: number) => {
        if (q + size > p.length) throw malformed();
        const value = uint(p, q, size, false);
        q += size;
        return value;
      };
      const count = take(version < 2 ? 2 : 4);
      for (let i = 0; i < count; i++) {
        const id = take(version < 2 ? 2 : 4);
        const method = version > 0 ? take(2) & 15 : 0;
        const reference = take(2);
        const base = take(baseSize);
        const extents = take(2);
        for (let e = 0; e < extents; e++) {
          take(indexSize);
          const offset = take(offsetSize);
          const length = take(lengthSize);
          if (!cleaned.has(id)) continue;
          if (method === 2 || reference !== 0 || length === 0 || method > 2) throw malformed();
          if (method === 0) edits.zero(base + offset, length);
          else {
            if (!idat || idat.start + idat.header + base + offset + length > idat.end) throw malformed();
            edits.zero(idat.start + idat.header + base + offset, length);
          }
        }
      }
    }
  }
  const moov = top.find((box) => box.type === "moov");
  if (moov) await cleanMoov(source, edits, moov, false);
}

// ---- JPEG XL container and JPEG 2000 ----------------------------------------------------

async function deleteTopBoxes(source: Source, edits: Edits, format: "jxl" | "jp2") {
  for (const box of await boxes(source, 0, source.size)) {
    let drop =
      format === "jxl" ? ["Exif", "xml ", "jumb"].includes(box.type) : ["uuid", "xml ", "uinf"].includes(box.type);
    if (format === "jxl" && box.type === "brob" && box.end - box.start >= box.header + 4)
      drop = ["Exif", "xml ", "jumb"].includes(fourcc(await source.read(box.start + box.header, 4), 0));
    if (drop) edits.replace(box.start, box.end - box.start, new Uint8Array());
  }
}

// ---- PSD -------------------------------------------------------------------------------

const PSD_DROPPED = new Set([0x0404, 0x0422, 0x0423, 0x0424, 0x0425]);

async function cleanPsd(source: Source, edits: Edits) {
  const colorModeLength = be32(await source.read(26, 4), 0);
  const start = 30 + colorModeLength;
  const length = be32(await source.read(start, 4), 0);
  const r = await source.read(start + 4, length);
  const kept: Uint8Array[] = [];
  let removed = false;
  for (let p = 0; p < r.length;) {
    if (p + 7 > r.length) throw malformed();
    const id = be16(r, p + 4);
    const nameLength = 1 + r[p + 6];
    const sizeAt = p + 6 + nameLength + (nameLength & 1);
    if (sizeAt + 4 > r.length) throw malformed();
    const size = be32(r, sizeAt);
    const end = sizeAt + 4 + size + (size & 1);
    if (end > r.length) throw malformed();
    if (PSD_DROPPED.has(id)) removed = true;
    else kept.push(r.subarray(p, end));
    p = end;
  }
  if (!removed) return;
  const body = concat(kept);
  edits.replace(
    start,
    4 + length,
    concat([
      Uint8Array.from([body.length >>> 24, body.length >> 16, body.length >> 8, body.length].map((v) => v & 0xff)),
      body,
    ]),
  );
}

// ---- TIFF family -------------------------------------------------------------------------

const TYPE_SIZES: Record<number, number> = {
  1: 1,
  2: 1,
  6: 1,
  7: 1,
  3: 2,
  8: 2,
  4: 4,
  9: 4,
  11: 4,
  13: 4,
  5: 8,
  10: 8,
  12: 8,
};
const IDENTIFYING_TAGS = new Set([315, 316, 700, 33723, 34377, 40093, 42016, 42032, 42033, 42037, 50735, 65000, 65001]);

interface Entry {
  tag: number;
  type: number;
  count: number;
  /** Where the value starts, or undefined for an unknown type. */
  data?: number;
  len: number;
  field: number;
}

class Tiff {
  constructor(
    readonly source: Source,
    readonly little: boolean,
    /** Offsets are relative to this. */
    readonly base: number,
    readonly end: number,
  ) {}

  u16 = (b: Uint8Array, o: number) => (this.little ? le16(b, o) : be16(b, o));
  u32 = (b: Uint8Array, o: number) => (this.little ? le32(b, o) : be32(b, o));

  /** Entries and next-IFD offset of the IFD at absolute position `at`. */
  async ifd(at: number): Promise<{ entries: Entry[]; next: number; size: number }> {
    if (at < 0 || at + 2 > this.end) throw malformed();
    const count = this.u16(await this.source.read(at, 2), 0);
    const size = 2 + count * 12 + 4;
    if (at + size > this.end) throw malformed();
    const raw = await this.source.read(at, size);
    const entries: Entry[] = [];
    for (let i = 0; i < count; i++) {
      const e = 2 + i * 12;
      const tag = this.u16(raw, e);
      const type = this.u16(raw, e + 2);
      const n = this.u32(raw, e + 4);
      const field = at + e + 8;
      const unit = TYPE_SIZES[type];
      const len = unit ? n * unit : 4;
      const data = !unit ? undefined : len <= 4 ? field : this.base + this.u32(raw, e + 8);
      entries.push({ tag, type, count: n, data, len, field });
    }
    return { entries, next: this.u32(raw, 2 + count * 12), size };
  }

  zeroValue(edits: Edits, entry: Entry) {
    if (entry.data === undefined) return edits.zero(entry.field, 4);
    if (entry.data < this.base || entry.data + entry.len > this.end) throw malformed();
    edits.zero(entry.data, entry.len);
  }

  async longs(entry: Entry) {
    if (entry.data === undefined || (entry.type !== 4 && entry.type !== 13)) return [];
    if (entry.data + entry.count * 4 > this.end) throw malformed();
    const raw = await this.source.read(entry.data, entry.count * 4);
    return Array.from({ length: entry.count }, (_, i) => this.base + this.u32(raw, i * 4));
  }

  async ascii(entry: Entry) {
    if (entry.data === undefined || entry.data + entry.len > this.end) return "";
    return String.fromCharCode(...(await this.source.read(entry.data, entry.len))).replace(/\0.*$/s, "");
  }
}

/** Cleans a TIFF structure in place: identifying tags, the GPS IFD and maker
 * note serials. `start..end` bounds it; offsets are relative to `start`. */
async function cleanTiff(source: Source, edits: Edits, start: number, end: number, top = true) {
  if (top) end = await cutCanonTrailer(source, edits);
  const head = await source.read(start, 8);
  const little = ascii(head, 0, "II");
  if (!little && !ascii(head, 0, "MM")) throw malformed();
  const tiff = new Tiff(source, little, start, end);
  const queue: { at: number; kind: "main" | "exif" | "gps" }[] = [{ at: start + tiff.u32(head, 4), kind: "main" }];
  const visited = new Set<number>();
  let make = "";
  const makerNotes: Entry[] = [];
  while (queue.length) {
    const { at, kind } = queue.shift()!;
    if (at === start || visited.has(at)) continue;
    visited.add(at);
    if (visited.size > 1000) throw malformed();
    const { entries, next, size } = await tiff.ifd(at);
    const removed: number[] = [];
    for (const [index, entry] of entries.entries()) {
      if (kind === "gps") {
        if (entry.data !== undefined && entry.data !== entry.field) tiff.zeroValue(edits, entry);
        continue;
      }
      if (IDENTIFYING_TAGS.has(entry.tag)) {
        if (entry.data !== undefined && entry.data !== entry.field) tiff.zeroValue(edits, entry);
        removed.push(index);
      } else if (entry.tag === 271 && kind === "main" && !make) make = await tiff.ascii(entry);
      else if (entry.tag === 330) for (const sub of await tiff.longs(entry)) queue.push({ at: sub, kind: "main" });
      else if (entry.tag === 34665) for (const sub of await tiff.longs(entry)) queue.push({ at: sub, kind: "exif" });
      else if (entry.tag === 34853) {
        // libtiff refuses an empty directory, so the pointer goes too.
        for (const sub of await tiff.longs(entry)) queue.push({ at: sub, kind: "gps" });
        removed.push(index);
      } else if (entry.tag === 37500 && kind === "exif") makerNotes.push(entry);
      else if (entry.tag === 50740 && kind === "main") makerNotes.push(entry);
      else if (entry.tag === 46 && kind === "main" && entry.data !== undefined && entry.data + entry.len <= end) {
        // Panasonic RW2 keeps Exif, GPS and maker notes in this embedded JPEG.
        if (ascii(await source.read(entry.data, 2), 0, "\xff\xd8"))
          await cleanEmbeddedJpeg(source, edits, entry.data, entry.data + entry.len);
      }
    }
    if (kind === "gps") edits.zero(at, size);
    else if (removed.length) {
      // Drop the entries in place: later entries and the next-IFD offset move
      // up, and the freed bytes at the end become zeros.
      const raw = await source.read(at, size);
      const out = new Uint8Array(size);
      const count = entries.length - removed.length;
      out.set(little ? [count & 0xff, count >> 8] : [count >> 8, count & 0xff], 0);
      let p = 2;
      for (let i = 0; i < entries.length; i++)
        if (!removed.includes(i)) {
          out.set(raw.subarray(2 + i * 12, 14 + i * 12), p);
          p += 12;
        }
      out.set(raw.subarray(size - 4), p);
      edits.replace(at, size, out);
    }
    if (kind === "main" && next) queue.push({ at: start + next, kind: "main" });
  }
  for (const note of makerNotes) {
    if (note.data === undefined || note.data + note.len > end) continue;
    const scratch = new Edits();
    try {
      let at = note.data;
      let len = note.len;
      let base = tiff.base;
      let little = tiff.little;
      if (note.tag === 50740) {
        // DNGPrivateData: Adobe's copy of the maker note, with its original
        // offset so TIFF-relative offsets still resolve, or (Pentax) the
        // maker note itself.
        const head = await source.read(at, Math.min(len, 20));
        if (ascii(head, 0, "Adobe\0MakN")) {
          // The maker note keeps the byte order of the file it came from.
          little = ascii(head, 14, "II");
          base = at + 20 - be32(head, 16);
          len = Math.min(be32(head, 10) - 6, len - 20);
          at += 20;
        }
      }
      await cleanMakerNote(new Tiff(source, little, base, end), scratch, at, len, make);
      edits.list.push(...scratch.list);
    } catch (error) {
      // A maker note this can't read stays as it is, as ExifTool leaves it.
      if (!(error instanceof MetadataRefused)) throw error;
    }
  }
}

interface MakerLayout {
  ifd: number;
  base: number;
  order?: number;
  little?: boolean;
  tags: number[];
  equipment?: number[];
  /** Tags of which only the first `n` value bytes identify the camera. */
  leading?: Record<number, number>;
}

async function cleanMakerNote(tiff: Tiff, edits: Edits, at: number, len: number, make: string) {
  const h = await tiff.source.read(at, Math.min(len, 24));
  const olympus = [1028, 4122];
  const equipment = [257, 258, 514, 770, 4099];
  let layout: MakerLayout | undefined;
  if (ascii(h, 0, "Nikon\0\x02")) layout = { ifd: at + 18, base: at + 10, order: at + 10, tags: [29, 160] };
  else if (ascii(h, 0, "FUJIFILM")) layout = { ifd: at + le32(h, 8), base: at, little: true, tags: [16] };
  else if (ascii(h, 0, "OLYMPUS\0")) layout = { ifd: at + 12, base: at, order: at + 8, tags: olympus, equipment };
  else if (ascii(h, 0, "OM SYSTEM\0\0\0"))
    layout = { ifd: at + 16, base: at, order: at + 12, tags: olympus, equipment };
  else if (ascii(h, 0, "OLYMP\0")) layout = { ifd: at + 8, base: tiff.base, tags: olympus, equipment };
  else if (ascii(h, 0, "Panasonic\0\0\0")) layout = { ifd: at + 12, base: tiff.base, tags: [37, 82, 84] };
  else if (ascii(h, 0, "AOC\0")) layout = { ifd: at + 6, base: at, order: at + 4, tags: [553] };
  else if (ascii(h, 0, "PENTAX \0")) layout = { ifd: at + 10, base: at, order: at + 8, tags: [553] };
  else if (["SONY DSC \0\0\0", "SONY CAM \0\0\0", "SONY MOBILE\0"].some((prefix) => ascii(h, 0, prefix)))
    layout = { ifd: at + 12, base: tiff.base, tags: [8241] };
  else if (make.startsWith("Canon"))
    layout = { ifd: at, base: tiff.base, tags: [9, 12, 40, 150], leading: { 16409: 5 } };
  else if (/^NIKON/i.test(make)) layout = { ifd: at, base: tiff.base, tags: [29, 160] };
  else if (make.startsWith("SONY")) layout = { ifd: at, base: tiff.base, tags: [8241] };
  if (!layout) return;
  let little = layout.little ?? tiff.little;
  if (layout.order !== undefined) {
    const order = await tiff.source.read(layout.order, 2);
    if (ascii(order, 0, "II")) little = true;
    else if (ascii(order, 0, "MM")) little = false;
  }
  const count = await tiff.source.read(layout.ifd, 2);
  const plausible = (value: number) => value > 0 && value <= 500;
  if (!plausible(little ? le16(count, 0) : be16(count, 0))) little = !little;
  const note = new Tiff(tiff.source, little, layout.base, tiff.end);
  const clean = async (ifd: number, tags: number[], sub?: number[]) => {
    for (const entry of (await note.ifd(ifd)).entries) {
      if (tags.includes(entry.tag)) note.zeroValue(edits, entry);
      else if (layout.leading?.[entry.tag] && entry.data !== undefined && entry.len >= layout.leading[entry.tag])
        note.zeroValue(edits, { ...entry, len: layout.leading[entry.tag] });
      else if (sub && entry.tag === 8208)
        await clean(layout.base + note.u32(await note.source.read(entry.field, 4), 0), sub);
    }
  };
  await clean(layout.ifd, layout.tags, layout.equipment);
}

// ---- RAF and CRW ---------------------------------------------------------------------------

async function cleanRaf(source: Source, edits: Edits) {
  const header = await source.read(84, 8);
  const start = be32(header, 0);
  await cleanEmbeddedJpeg(source, edits, start, start + be32(header, 4));
}

/** A JPEG inside a raw (RAF, RW2), cleaned in place up to its first scan: its
 * Exif TIFF by the TIFF rules, its XMP, IPTC and comment payloads zeroed. */
async function cleanEmbeddedJpeg(source: Source, edits: Edits, start: number, end: number) {
  if (end > source.size || !ascii(await source.read(start, 2), 0, "\xff\xd8")) throw malformed();
  for (let p = start + 2; p + 4 <= end;) {
    const h = await source.read(p, 4);
    if (h[0] !== 0xff) throw malformed();
    const marker = h[1];
    if (marker === 0xda || marker === 0xd9) break;
    const segmentEnd = p + 2 + be16(h, 2);
    if (be16(h, 2) < 2 || segmentEnd > end) throw malformed();
    const payload = p + 4;
    const tag = await source.peek(payload, 29);
    if (marker === 0xe1 && ascii(tag, 0, "Exif\0\0")) await cleanTiff(source, edits, payload + 6, segmentEnd, false);
    else if ((marker === 0xe1 && ascii(tag, 0, "http://ns.adobe.com/xap/1.0/\0")) || marker === 0xed || marker === 0xfe)
      edits.zero(payload, segmentEnd - payload);
    p = segmentEnd;
  }
}

const VRD = "CANON OPTIONAL DATA\0";

/** Cuts off a Canon DPP (VRD) trailer, where recipes and XMP are kept, and
 * returns where the file now ends. */
async function cutCanonTrailer(source: Source, edits: Edits) {
  if (source.size < 0x5c) return source.size;
  const footer = await source.read(source.size - 0x40, 0x40);
  if (!ascii(footer, 0, VRD)) return source.size;
  let start = source.size - 0x40 - be32(footer, 0x14) - 0x1c;
  if (start < 0 || !ascii(await source.read(start, VRD.length), 0, VRD)) throw malformed();
  // ExifTool word-aligns the trailer with one padding byte; it goes too.
  if (start % 2) start--;
  edits.replace(start, source.size - start, new Uint8Array());
  return start;
}

async function cleanCrw(source: Source, edits: Edits) {
  let heaps = 0;
  const heap = async (start: number, end: number) => {
    if (++heaps > 1000 || end - start < 4) throw malformed();
    const table = start + le32(await source.read(end - 4, 4), 0);
    const count = le16(await source.read(table, 2), 0);
    const records = await source.read(table + 2, count * 10);
    for (let i = 0; i < count; i++) {
      const type = le16(records, i * 10);
      const size = le32(records, i * 10 + 2);
      const offset = le32(records, i * 10 + 6);
      const inRecord = (type & 0xc000) === 0x4000;
      const data = inRecord ? table + 2 + i * 10 + 2 : start + offset;
      const len = inRecord ? 8 : size;
      if (!inRecord && data + len > end) throw malformed();
      const id = type & 0x3fff;
      if (!inRecord && [0x28, 0x30].includes((type >> 8) & 0x38)) await heap(data, data + len);
      else if (id === 0x0810 || id === 0x180b) edits.zero(data, len);
    }
  };
  await heap(le32(await source.read(2, 4), 0), await cutCanonTrailer(source, edits));
}

// ---- Matroska / WebM -----------------------------------------------------------------------

interface Element {
  id: number;
  start: number;
  data: number;
  /** Undefined for an unknown size. */
  end?: number;
}

const SEGMENT_CHILDREN = new Set([
  0x1f43b675, 0x1c53bb6b, 0x1254c367, 0x1941a469, 0x1043a770, 0x1549a966, 0x1654ae6b, 0x114d9b74,
]);

async function element(source: Source, at: number): Promise<Element> {
  const head = await source.peek(at, 12);
  const idWidth = Math.clz32(head[0]) - 23;
  if (idWidth < 1 || idWidth > 4 || head.length < idWidth + 1) throw malformed();
  const id = uint(head, 0, idWidth, false);
  const first = head[idWidth];
  const width = Math.clz32(first) - 23;
  if (width < 1 || width > 8 || head.length < idWidth + width) throw malformed();
  let size = first & (0xff >> width);
  let allOnes = size === 0xff >> width;
  for (let i = 1; i < width; i++) {
    size = size * 256 + head[idWidth + i];
    allOnes &&= head[idWidth + i] === 0xff;
  }
  const data = at + idWidth + width;
  return { id, start: at, data, end: allOnes ? undefined : data + size };
}

function voidElement(edits: Edits, start: number, end: number) {
  const length = end - start;
  let width = 1;
  while (length - 1 - width > 2 ** (7 * width) - 2) width++;
  const size = length - 1 - width;
  const header = new Uint8Array(1 + width);
  header[0] = 0xec;
  let value = size;
  for (let i = width; i >= 1; i--) {
    header[i] = value % 256;
    value = Math.floor(value / 256);
  }
  header[1] |= 0x80 >> (width - 1);
  edits.replace(start, 1 + width, header);
  edits.zero(start + 1 + width, size);
}

async function childElements(source: Source, parent: { data: number; end: number }) {
  const list: (Element & { end: number })[] = [];
  for (let at = parent.data; at < parent.end;) {
    const child = await element(source, at);
    if (child.end === undefined || child.end > parent.end) throw malformed();
    list.push(child as Element & { end: number });
    at = child.end;
  }
  return list;
}

async function elementUint(source: Source, child: { data: number; end: number }) {
  const bytes = await source.read(child.data, child.end - child.data);
  return uint(bytes, 0, Math.min(bytes.length, 6), false);
}

/** Voids `ids` among `parent`'s children, and its CRC-32 when anything went. */
async function voidChildren(source: Source, edits: Edits, parent: { data: number; end: number }, ids: number[]) {
  const children = await childElements(source, parent);
  const doomed = children.filter((child) => ids.includes(child.id));
  for (const child of doomed) voidElement(edits, child.start, child.end);
  if (doomed.length)
    for (const crc of children.filter((child) => child.id === 0xbf)) voidElement(edits, crc.start, crc.end);
}

async function cleanMatroska(source: Source, edits: Edits) {
  for (let at = 0; at < source.size;) {
    const top = await element(source, at);
    if (top.id !== 0x18538067) {
      if (top.end === undefined) throw malformed();
      at = top.end;
      continue;
    }
    const end = Math.min(top.end ?? source.size, source.size);
    if (top.end !== undefined && top.end > source.size) throw malformed();
    // Children, finding where unknown-size clusters end.
    const children: (Element & { end: number })[] = [];
    for (let p = top.data; p < end;) {
      const child = await element(source, p);
      if (child.end !== undefined) {
        if (child.end > end) throw malformed();
        children.push(child as Element & { end: number });
        p = child.end;
        continue;
      }
      if (child.id !== 0x1f43b675) throw malformed();
      let q = child.data;
      while (q < end) {
        const inner = await element(source, q);
        if (SEGMENT_CHILDREN.has(inner.id) || inner.end === undefined) break;
        if (inner.end > end) throw malformed();
        q = inner.end;
      }
      children.push({ ...child, end: q });
      p = q;
    }
    const dropped = new Set<number>();
    let modified = false;
    for (const child of children) {
      if ([0x1254c367, 0x1941a469, 0x1043a770].includes(child.id)) {
        voidElement(edits, child.start, child.end);
        modified = true;
      } else if (child.id === 0x1549a966) {
        await voidChildren(source, edits, child, [0x7ba9, 0x4461]);
      } else if (child.id === 0x1654ae6b) {
        const entries = await childElements(source, child);
        let voided = false;
        for (const entry of entries.filter((e) => e.id === 0xae)) {
          const fields = await childElements(source, entry);
          const type = fields.find((f) => f.id === 0x83);
          const trackType = type ? await elementUint(source, type) : 0;
          if (trackType !== 1 && trackType !== 2) {
            const number = fields.find((f) => f.id === 0xd7);
            if (number) dropped.add(await elementUint(source, number));
            voidElement(edits, entry.start, entry.end);
            voided = true;
          } else {
            await voidChildren(source, edits, entry, [0x536e]);
          }
        }
        if (voided) for (const crc of entries.filter((e) => e.id === 0xbf)) voidElement(edits, crc.start, crc.end);
      }
    }
    if (dropped.size)
      for (const cluster of children.filter((child) => child.id === 0x1f43b675)) {
        const blocks = await childElements(source, cluster);
        let voided = false;
        for (const block of blocks) {
          let payload: number | undefined;
          if (block.id === 0xa3) payload = block.data;
          else if (block.id === 0xa0) payload = (await childElements(source, block)).find((e) => e.id === 0xa1)?.data;
          if (payload === undefined) continue;
          const head = await source.peek(payload, 8);
          const width = Math.clz32(head[0]) - 23;
          if (width < 1 || width > 8) throw malformed();
          const track = uint(head, 0, width, false) - 2 ** (8 * width - width);
          if (dropped.has(track)) {
            voidElement(edits, block.start, block.end);
            voided = true;
          }
        }
        if (voided) for (const crc of blocks.filter((e) => e.id === 0xbf)) voidElement(edits, crc.start, crc.end);
      }
    if (modified)
      for (const crc of children.filter((child) => child.id === 0xbf)) voidElement(edits, crc.start, crc.end);
    at = end;
  }
}

// ---- RIFF: WAV, AIFF and AVI -------------------------------------------------------------

interface Chunk {
  id: string;
  start: number;
  size: number;
  end: number;
}

async function riffChunks(source: Source, start: number, end: number, big: boolean, tolerant: string[]) {
  const list: Chunk[] = [];
  for (let p = start; p < end;) {
    if (p + 8 > end) break;
    const h = await source.read(p, 8);
    const id = fourcc(h, 0);
    const size = big ? be32(h, 4) : le32(h, 4);
    let chunkEnd = p + 8 + size + (size & 1);
    if (p + 8 + size > end) {
      if (!tolerant.includes(id)) throw malformed();
      list.push({ id, start: p, size: end - p - 8, end });
      break;
    }
    chunkEnd = Math.min(chunkEnd, end);
    list.push({ id, start: p, size, end: chunkEnd });
    p = chunkEnd;
  }
  return list;
}

function junk(edits: Edits, chunk: Chunk, id: string) {
  edits.replace(chunk.start, 4, text(id));
  edits.zero(chunk.start + 8, chunk.size);
}

const WAV_KEPT = [
  "fmt ",
  "data",
  "fact",
  "cue ",
  "smpl",
  "inst",
  "plst",
  "acid",
  "ds64",
  "JUNK",
  "junk",
  "PAD ",
  "FLLR",
];
const AIFF_KEPT = ["COMM", "SSND", "FVER", "CHAN", "FLLR"];

async function cleanRiffAudio(source: Source, edits: Edits, aiff: boolean) {
  const kept = aiff ? AIFF_KEPT : WAV_KEPT;
  for (const chunk of await riffChunks(source, 12, source.size, aiff, [aiff ? "SSND" : "data"]))
    if (!kept.includes(chunk.id)) junk(edits, chunk, aiff ? "FLLR" : "JUNK");
}

async function listType(source: Source, chunk: Chunk) {
  return chunk.id === "LIST" && chunk.size >= 4 ? `LIST ${fourcc(await source.read(chunk.start + 8, 4), 0)}` : chunk.id;
}

async function cleanAvi(source: Source, edits: Edits) {
  const top = await riffChunks(source, 0, source.size, false, ["RIFF"]);
  const dropped = new Set<string>();
  const movis: Chunk[] = [];
  for (const [index, riff] of top.entries()) {
    const form = fourcc(await source.read(riff.start + 8, 4), 0);
    if (riff.id !== "RIFF" || (index === 0 ? form !== "AVI " : form !== "AVIX")) continue;
    for (const chunk of await riffChunks(source, riff.start + 12, riff.start + 8 + riff.size, false, ["LIST"])) {
      const kind = await listType(source, chunk);
      if (kind === "LIST movi") movis.push(chunk);
      if (index > 0) continue;
      if (kind === "LIST hdrl") {
        let stream = 0;
        for (const header of await riffChunks(source, chunk.start + 12, chunk.start + 8 + chunk.size, false, [])) {
          const headerKind = await listType(source, header);
          if (headerKind === "LIST strl") {
            for (const part of await riffChunks(source, header.start + 12, header.start + 8 + header.size, false, [])) {
              if (part.id === "strh" && part.size >= 4) {
                const type = fourcc(await source.read(part.start + 8, 4), 0);
                if (type !== "vids" && type !== "auds") dropped.add(String(stream).padStart(2, "0"));
              }
              if (!["strh", "strf", "strd", "indx", "vprp", "JUNK"].includes(part.id)) junk(edits, part, "JUNK");
            }
            stream++;
          } else if (!["avih", "LIST odml", "JUNK"].includes(headerKind)) {
            junk(edits, header, "JUNK");
          }
        }
      } else if (!["LIST movi", "idx1", "JUNK"].includes(kind)) {
        junk(edits, chunk, "JUNK");
      }
    }
  }
  if (!dropped.size) return;
  const walk = async (start: number, end: number) => {
    for (const chunk of await riffChunks(source, start, end, false, [])) {
      if ((await listType(source, chunk)) === "LIST rec ") await walk(chunk.start + 12, chunk.start + 8 + chunk.size);
      else if (dropped.has(chunk.id.slice(0, 2))) junk(edits, chunk, "JUNK");
    }
  };
  for (const movi of movis) await walk(movi.start + 12, movi.start + 8 + movi.size);
}

// ---- MPEG audio, ID3 and FLAC -----------------------------------------------------------------

const syncsafe = (b: Uint8Array, o: number) =>
  b.subarray(o, o + 4).every((byte) => byte < 0x80) ? (b[o] << 21) | (b[o + 1] << 14) | (b[o + 2] << 7) | b[o + 3] : -1;

async function cleanMpeg(source: Source, edits: Edits) {
  let p = 0;
  for (;;) {
    const h = await source.peek(p, 10);
    if (!(h.length === 10 && ascii(h, 0, "ID3") && h[3] >= 2 && h[3] <= 4)) break;
    const size = syncsafe(h, 6);
    if (size < 0) break;
    const total = 10 + size + (h[3] === 4 && h[5] & 0x10 ? 10 : 0);
    if (p + total > source.size) throw malformed();
    const header = Uint8Array.from([0x49, 0x44, 0x33, h[3], h[4], 0, ...syncsafeBytes(total - 10)]);
    edits.replace(p, 10, header);
    edits.zero(p + 10, total - 10);
    p += total;
  }
  if (ascii(await source.peek(p, 4), 0, "fLaC")) p = await cleanFlac(source, edits, p);
  await cleanTrailingTags(source, edits, p);
}

const syncsafeBytes = (n: number) => [(n >> 21) & 0x7f, (n >> 14) & 0x7f, (n >> 7) & 0x7f, n & 0x7f];

/** Cuts ID3v1, Enhanced, APEv2, ID3v2 footer and Lyrics3v2 tags off the end
 * (zeros there would be decoded as broken frames), never below `floor`. */
async function cleanTrailingTags(source: Source, edits: Edits, floor: number) {
  let end = source.size;
  const take = (length: number) => {
    if (length < 1 || end - length < floor) throw malformed();
    end -= length;
    edits.replace(end, length, new Uint8Array());
  };
  for (;;) {
    const tail = await source.read(Math.max(floor, end - 227), end - Math.max(floor, end - 227));
    const at = (back: number, value: string) => tail.length >= back && ascii(tail, tail.length - back, value);
    if (at(128, "TAG")) {
      take(128);
      const enhanced = await source.read(Math.max(floor, end - 227), end - Math.max(floor, end - 227));
      if (enhanced.length >= 227 && ascii(enhanced, enhanced.length - 227, "TAG+")) take(227);
    } else if (at(32, "APETAGEX")) {
      const footer = tail.subarray(tail.length - 32);
      take(le32(footer, 12) + (le32(footer, 20) & 0x80000000 ? 32 : 0));
    } else if (at(10, "3DI")) {
      const size = syncsafe(tail, tail.length - 4);
      if (size < 0) throw malformed();
      take(20 + size);
    } else if (at(9, "LYRICS200")) {
      const digits = String.fromCharCode(...tail.subarray(tail.length - 15, tail.length - 9));
      if (!/^\d{6}$/.test(digits)) throw malformed();
      take(Number(digits) + 15);
    } else {
      return;
    }
  }
}

/** FLAC metadata blocks after `fLaC` at `at`; returns where audio frames start. */
async function cleanFlac(source: Source, edits: Edits, at: number) {
  let p = at + 4;
  for (;;) {
    const h = await source.read(p, 4);
    const type = h[0] & 0x7f;
    const length = (h[1] << 16) | (h[2] << 8) | h[3];
    if (type === 127 || p + 4 + length > source.size) throw malformed();
    if (type !== 0 && type !== 3) {
      edits.replace(p, 1, Uint8Array.of((h[0] & 0x80) | 1));
      edits.zero(p + 4, length);
    }
    p += 4 + length;
    if (h[0] & 0x80) return p;
  }
}

// ---- Ogg ---------------------------------------------------------------------------------

let oggTable: Uint32Array | undefined;
function oggCrc(page: Uint8Array) {
  oggTable ??= Uint32Array.from({ length: 256 }, (_, n) => {
    let c = n << 24;
    for (let k = 0; k < 8; k++) c = c & 0x80000000 ? (c << 1) ^ 0x04c11db7 : c << 1;
    return c >>> 0;
  });
  let crc = 0;
  for (const byte of page) crc = ((crc << 8) ^ oggTable[((crc >>> 24) ^ byte) & 0xff]) >>> 0;
  return crc;
}

interface OggPage {
  start: number;
  bytes: Uint8Array;
  changed: boolean;
}

interface OggStream {
  codec?: "vorbis" | "opus" | "theora" | "speex" | "flac" | "other";
  index: number;
  /** Byte ranges (page, offset in page, length) of the packet being read. */
  pending: { page: OggPage; offset: number; length: number }[];
  done: boolean;
}

function rewriteComments(packet: Uint8Array, prefix: number, framing: boolean) {
  if (packet.length < prefix + 4) throw malformed();
  const vendor = le32(packet, prefix);
  const needed = prefix + 4 + vendor + 4 + (framing ? 1 : 0);
  if (needed > packet.length) throw malformed();
  const out = new Uint8Array(packet.length);
  out.set(packet.subarray(0, prefix + 4 + vendor));
  if (framing) out[prefix + 4 + vendor + 4] = 1;
  return out;
}

function cleanFlacBlock(packet: Uint8Array) {
  const type = packet[0] & 0x7f;
  if (type === 127) throw malformed();
  if (type === 0 || type === 3) return { packet, last: (packet[0] & 0x80) !== 0 };
  const out = new Uint8Array(packet.length);
  out.set(packet.subarray(0, 4));
  out[0] = (packet[0] & 0x80) | 1;
  return { packet: out, last: (packet[0] & 0x80) !== 0 };
}

async function cleanOgg(source: Source, edits: Edits) {
  const streams = new Map<number, OggStream>();
  const changed: OggPage[] = [];
  let p = 0;
  while (p + 27 <= source.size) {
    const head = await source.read(p, 27);
    if (!ascii(head, 0, "OggS") || head[4] !== 0) {
      // Resynchronize at the next capture pattern.
      let q = p + 1;
      for (;;) {
        const window = await source.peek(q, CHUNK);
        if (window.length < 4) return finishOgg(edits, changed);
        let found = -1;
        for (let i = 0; i + 4 <= window.length && found < 0; i++) if (ascii(window, i, "OggS")) found = i;
        if (found >= 0) {
          q += found;
          break;
        }
        q += window.length - 3;
      }
      p = q;
      continue;
    }
    const segments = head[26];
    if (p + 27 + segments > source.size) break;
    const lacing = await source.read(p + 27, segments);
    const bodyLength = lacing.reduce((sum, value) => sum + value, 0);
    const length = 27 + segments + bodyLength;
    if (p + length > source.size) break;
    const serial = le32(head, 14);
    if (head[5] & 2) streams.set(serial, { index: 0, pending: [], done: false });
    const stream = streams.get(serial);
    if (stream && !stream.done) {
      const page: OggPage = { start: p, bytes: (await source.read(p, length)).slice(), changed: false };
      if (!(head[5] & 1)) stream.pending = [];
      let offset = 27 + segments;
      for (const value of lacing) {
        stream.pending.push({ page, offset, length: value });
        offset += value;
        if (value < 255) {
          handleOggPacket(stream, changed);
          stream.pending = [];
          stream.index++;
          if (stream.done) break;
        }
      }
    }
    p += length;
  }
  finishOgg(edits, changed);
}

function handleOggPacket(stream: OggStream, changed: OggPage[]) {
  const packet = concat(stream.pending.map((part) => part.page.bytes.subarray(part.offset, part.offset + part.length)));
  let replacement: Uint8Array | undefined;
  if (stream.index === 0) {
    if (ascii(packet, 0, "\x01vorbis")) stream.codec = "vorbis";
    else if (ascii(packet, 0, "OpusHead")) stream.codec = "opus";
    else if (ascii(packet, 0, "\x80theora")) stream.codec = "theora";
    else if (ascii(packet, 0, "Speex   ")) stream.codec = "speex";
    else if (ascii(packet, 0, "\x7fFLAC")) stream.codec = "flac";
    else stream.done = true;
    return;
  }
  switch (stream.codec) {
    case "vorbis":
      if (!ascii(packet, 0, "\x03vorbis")) throw malformed();
      replacement = rewriteComments(packet, 7, true);
      stream.done = true;
      break;
    case "opus":
      if (!ascii(packet, 0, "OpusTags")) throw malformed();
      replacement = rewriteComments(packet, 8, false);
      stream.done = true;
      break;
    case "theora":
      if (!ascii(packet, 0, "\x81theora")) throw malformed();
      replacement = rewriteComments(packet, 7, false);
      stream.done = true;
      break;
    case "speex":
      replacement = rewriteComments(packet, 0, false);
      stream.done = true;
      break;
    case "flac": {
      if (packet.length < 4) throw malformed();
      const block = cleanFlacBlock(packet);
      replacement = block.packet;
      stream.done = block.last;
      break;
    }
    default:
      stream.done = true;
  }
  if (!replacement || replacement.every((byte, i) => byte === packet[i])) return;
  let at = 0;
  for (const part of stream.pending) {
    part.page.bytes.set(replacement.subarray(at, at + part.length), part.offset);
    at += part.length;
    if (!part.page.changed) {
      part.page.changed = true;
      changed.push(part.page);
    }
  }
}

function finishOgg(edits: Edits, pages: OggPage[]) {
  for (const page of pages) {
    page.bytes.fill(0, 22, 26);
    const crc = oggCrc(page.bytes);
    page.bytes.set([crc & 0xff, (crc >> 8) & 0xff, (crc >> 16) & 0xff, (crc >>> 24) & 0xff], 22);
    edits.replace(page.start, page.bytes.length, page.bytes);
  }
}
