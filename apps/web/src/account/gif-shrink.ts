// Shrinking an animated GIF too large to upload unchanged: every frame is
// composited, center-cropped square, scaled down, mapped to one shared palette
// and re-encoded with its timing. Pure (no DOM), so it runs in a worker and in
// tests. Android's GifShrink.kt is the same algorithm.

/** A shrunk GIF's edge, like a re-encoded still's. */
export const SHRINK_EDGE = 512;

export class GifShrinkError extends Error {}

interface Header {
  width: number;
  height: number;
  palette?: Uint8Array;
  /** Where the first block after the header (and global palette) starts. */
  start: number;
}

function header(bytes: Uint8Array): Header {
  const signature = String.fromCharCode(...bytes.subarray(0, 6));
  if (bytes.length < 13 || (signature !== "GIF87a" && signature !== "GIF89a")) throw new GifShrinkError("not a GIF");
  const width = bytes[6] | (bytes[7] << 8);
  const height = bytes[8] | (bytes[9] << 8);
  if (!width || !height) throw new GifShrinkError("empty GIF");
  const packed = bytes[10];
  let start = 13;
  let palette: Uint8Array | undefined;
  if (packed & 0x80) {
    const size = 3 * (2 << (packed & 7));
    palette = bytes.subarray(start, start + size);
    start += size;
  }
  return { width, height, palette, start };
}

/** The data sub-blocks starting at `at`, joined; and where they end. */
function subBlocks(bytes: Uint8Array, at: number): { data: Uint8Array; end: number } {
  let length = 0;
  let cursor = at;
  while (cursor < bytes.length && bytes[cursor] !== 0) {
    length += bytes[cursor];
    cursor += bytes[cursor] + 1;
  }
  const data = new Uint8Array(length);
  let offset = 0;
  for (let block = at; block < cursor && block < bytes.length; block += bytes[block] + 1) {
    const part = bytes.subarray(block + 1, Math.min(bytes.length, block + 1 + bytes[block]));
    data.set(part, offset);
    offset += part.length;
  }
  return { data: data.subarray(0, offset), end: cursor + 1 };
}

/** GIF LZW: codes from `data` into `out` (left as is where the data ends early). */
function lzwDecode(data: Uint8Array, minSize: number, out: Uint8Array) {
  if (minSize < 2 || minSize > 11) return;
  const clear = 1 << minSize;
  const end = clear + 1;
  const prefix = new Int16Array(4096);
  const suffix = new Uint8Array(4096);
  const stack = new Uint8Array(4097);
  for (let code = 0; code < clear; code++) suffix[code] = code;
  let size = minSize + 1;
  let mask = (1 << size) - 1;
  let next = clear + 2;
  let old = -1;
  let first = 0;
  let bits = 0;
  let accumulator = 0;
  let written = 0;
  for (let at = 0; at < data.length && written < out.length; at++) {
    accumulator |= data[at] << bits;
    bits += 8;
    while (bits >= size) {
      let code = accumulator & mask;
      accumulator >>>= size;
      bits -= size;
      if (code === clear) {
        size = minSize + 1;
        mask = (1 << size) - 1;
        next = clear + 2;
        old = -1;
        continue;
      }
      if (code === end) return;
      if (old === -1) {
        if (code >= clear) return;
        out[written++] = code;
        old = first = code;
        continue;
      }
      if (code > next) return;
      const current = code;
      let depth = 0;
      if (code === next) {
        stack[depth++] = first;
        code = old;
      }
      while (code >= clear && depth < 4096) {
        stack[depth++] = suffix[code];
        code = prefix[code];
      }
      first = code;
      stack[depth++] = first;
      while (depth > 0 && written < out.length) out[written++] = stack[--depth];
      if (next < 4096) {
        prefix[next] = old;
        suffix[next] = first;
        next++;
        if (next === mask + 1 && size < 12) {
          size++;
          mask = (1 << size) - 1;
        }
      }
      old = current;
    }
  }
}

/**
 * Every frame of `bytes` as the full, composited RGBA canvas a viewer sees,
 * with its delay in hundredths of a second. `frame` must use the canvas before
 * returning: it is reused. Returns the frame count.
 */
export function decodeGif(
  bytes: Uint8Array,
  frame: (rgba: Uint8ClampedArray, delay: number, index: number) => void,
): number {
  const { width, height, palette: global, start } = header(bytes);
  const canvas = new Uint8ClampedArray(width * height * 4);
  let saved: Uint8ClampedArray | undefined;
  let delay = 0;
  let transparent = -1;
  let disposal = 0;
  // The previous frame's rectangle and disposal, applied before the next draws.
  let previous: { x: number; y: number; w: number; h: number; disposal: number } | undefined;
  let count = 0;
  let at = start;
  while (at < bytes.length) {
    const block = bytes[at++];
    if (block === 0x3b) break;
    if (block === 0x21) {
      const label = bytes[at++];
      if (label === 0xf9 && bytes[at] >= 4) {
        const packed = bytes[at + 1];
        disposal = (packed >> 2) & 7;
        delay = bytes[at + 2] | (bytes[at + 3] << 8);
        transparent = packed & 1 ? bytes[at + 4] : -1;
      }
      at = subBlocks(bytes, at).end;
      continue;
    }
    if (block !== 0x2c) break;
    if (at + 9 > bytes.length) break;
    const x = bytes[at] | (bytes[at + 1] << 8);
    const y = bytes[at + 2] | (bytes[at + 3] << 8);
    const w = bytes[at + 4] | (bytes[at + 5] << 8);
    const h = bytes[at + 6] | (bytes[at + 7] << 8);
    const packed = bytes[at + 8];
    at += 9;
    let palette = global;
    if (packed & 0x80) {
      const size = 3 * (2 << (packed & 7));
      palette = bytes.subarray(at, at + size);
      at += size;
    }
    const minSize = bytes[at++];
    const { data, end } = subBlocks(bytes, at);
    at = end;
    if (previous?.disposal === 2) clearRect(canvas, width, height, previous);
    else if (previous?.disposal === 3 && saved) canvas.set(saved);
    if (disposal === 3) saved = canvas.slice();
    const indices = new Uint8Array(w * h);
    if (transparent >= 0) indices.fill(transparent);
    lzwDecode(data, minSize, indices);
    if (palette) draw(canvas, width, height, { x, y, w, h }, indices, palette, transparent, (packed & 0x40) !== 0);
    frame(canvas, delay, count++);
    previous = { x, y, w, h, disposal };
    delay = 0;
    transparent = -1;
    disposal = 0;
  }
  if (!count) throw new GifShrinkError("no frames");
  return count;
}

type Rect = { x: number; y: number; w: number; h: number };

function clearRect(canvas: Uint8ClampedArray, width: number, height: number, rect: Rect) {
  for (let row = rect.y; row < Math.min(height, rect.y + rect.h); row++)
    canvas.fill(0, (row * width + rect.x) * 4, (row * width + Math.min(width, rect.x + rect.w)) * 4);
}

/** Interlaced images store rows 0, 8, 16… then 4, 12… then 2, 6… then 1, 3…. */
function rowOrder(h: number, interlaced: boolean) {
  if (!interlaced) return Array.from({ length: h }, (_, row) => row);
  const rows: number[] = [];
  for (const [first, step] of [
    [0, 8],
    [4, 8],
    [2, 4],
    [1, 2],
  ])
    for (let row = first; row < h; row += step) rows.push(row);
  return rows;
}

function draw(
  canvas: Uint8ClampedArray,
  width: number,
  height: number,
  rect: Rect,
  indices: Uint8Array,
  palette: Uint8Array,
  transparent: number,
  interlaced: boolean,
) {
  const rows = rowOrder(rect.h, interlaced);
  for (let line = 0; line < rect.h; line++) {
    const y = rect.y + rows[line];
    if (y >= height) continue;
    for (let column = 0; column < rect.w; column++) {
      const x = rect.x + column;
      if (x >= width) break;
      const index = indices[line * rect.w + column];
      if (index === transparent || index * 3 + 2 >= palette.length) continue;
      const to = (y * width + x) * 4;
      canvas[to] = palette[index * 3];
      canvas[to + 1] = palette[index * 3 + 1];
      canvas[to + 2] = palette[index * 3 + 2];
      canvas[to + 3] = 255;
    }
  }
}

/**
 * The centered square of `source` (width × height) averaged down to edge ×
 * edge. Colour is weighted by alpha so transparent edges don't darken; a
 * pixel is opaque when at least half its area was.
 */
export function squareScale(
  source: Uint8ClampedArray,
  width: number,
  height: number,
  edge: number,
  out: Uint8ClampedArray,
) {
  const side = Math.min(width, height);
  const left = Math.floor((width - side) / 2);
  const top = Math.floor((height - side) / 2);
  const span = (index: number) => {
    const from = Math.floor((index * side) / edge);
    return [from, Math.max(from + 1, Math.floor(((index + 1) * side) / edge))];
  };
  const spans = Array.from({ length: edge }, (_, index) => span(index));
  for (let oy = 0; oy < edge; oy++) {
    const [y0, y1] = spans[oy];
    for (let ox = 0; ox < edge; ox++) {
      const [x0, x1] = spans[ox];
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      for (let y = y0; y < y1; y++) {
        let at = ((top + y) * width + left + x0) * 4;
        for (let x = x0; x < x1; x++, at += 4) {
          const alpha = source[at + 3];
          r += source[at] * alpha;
          g += source[at + 1] * alpha;
          b += source[at + 2] * alpha;
          a += alpha;
        }
      }
      const to = (oy * edge + ox) * 4;
      const area = (y1 - y0) * (x1 - x0);
      if (a * 2 < area * 255) {
        out[to + 3] = 0;
        continue;
      }
      out[to] = r / a;
      out[to + 1] = g / a;
      out[to + 2] = b / a;
      out[to + 3] = 255;
    }
  }
}

/** Colours are binned at 5 bits per channel; the palette is a median cut of the bins. */
const bin = (r: number, g: number, b: number) => ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);

class Histogram {
  private readonly counts = new Float64Array(32768);
  private readonly sums = new Float64Array(32768 * 3);
  private readonly used: number[] = [];

  add(r: number, g: number, b: number) {
    const key = bin(r, g, b);
    if (!this.counts[key]) this.used.push(key);
    this.counts[key]++;
    this.sums[key * 3] += r;
    this.sums[key * 3 + 1] += g;
    this.sums[key * 3 + 2] += b;
  }

  /** Up to `size` colours (RGB triples), each box's weighted mean; then empties the histogram. */
  palette(size: number): Uint8Array {
    const channel = (key: number, axis: number) => (key >> (10 - axis * 5)) & 31;
    const box = (keys: number[]) => {
      let weight = 0;
      let range = 0;
      let axis = 0;
      for (const key of keys) weight += this.counts[key];
      for (let a = 0; a < 3; a++) {
        let low = 31;
        let high = 0;
        for (const key of keys) {
          const value = channel(key, a);
          if (value < low) low = value;
          if (value > high) high = value;
        }
        if (high - low > range) {
          range = high - low;
          axis = a;
        }
      }
      return { keys, weight, range, axis };
    };
    const boxes = this.used.length ? [box(this.used.slice())] : [];
    while (boxes.length < size) {
      let chosen = -1;
      let score = 0;
      for (let index = 0; index < boxes.length; index++) {
        const candidate = boxes[index];
        const value = candidate.weight * (candidate.range + 1);
        if (candidate.range > 0 && value > score) {
          score = value;
          chosen = index;
        }
      }
      if (chosen < 0) break;
      const { keys, weight, axis } = boxes[chosen];
      keys.sort((a, b) => channel(a, axis) - channel(b, axis));
      let running = 0;
      let cut = 1;
      for (; cut < keys.length - 1; cut++) {
        running += this.counts[keys[cut - 1]];
        if (running >= weight / 2) break;
      }
      boxes.splice(chosen, 1, box(keys.slice(0, cut)), box(keys.slice(cut)));
    }
    const palette = new Uint8Array(Math.max(1, boxes.length) * 3);
    boxes.forEach(({ keys, weight }, index) => {
      let r = 0;
      let g = 0;
      let b = 0;
      for (const key of keys) {
        r += this.sums[key * 3];
        g += this.sums[key * 3 + 1];
        b += this.sums[key * 3 + 2];
      }
      palette[index * 3] = Math.round(r / weight);
      palette[index * 3 + 1] = Math.round(g / weight);
      palette[index * 3 + 2] = Math.round(b / weight);
    });
    for (const key of this.used) {
      this.counts[key] = 0;
      this.sums.fill(0, key * 3, key * 3 + 3);
    }
    this.used.length = 0;
    return palette;
  }
}

/** Nearest palette entry per colour bin, found once per bin. */
class Mapper {
  private readonly cache = new Int16Array(32768).fill(-1);
  private readonly palette: Uint8Array;

  constructor(palette: Uint8Array) {
    this.palette = palette;
  }

  index(r: number, g: number, b: number) {
    const key = bin(r, g, b);
    const cached = this.cache[key];
    if (cached >= 0) return cached;
    const cr = ((key >> 10) << 3) | 4;
    const cg = (((key >> 5) & 31) << 3) | 4;
    const cb = ((key & 31) << 3) | 4;
    let best = 0;
    let distance = Infinity;
    for (let entry = 0; entry * 3 < this.palette.length; entry++) {
      const dr = this.palette[entry * 3] - cr;
      const dg = this.palette[entry * 3 + 1] - cg;
      const db = this.palette[entry * 3 + 2] - cb;
      const d = dr * dr * 2 + dg * dg * 4 + db * db * 3;
      if (d < distance) {
        distance = d;
        best = entry;
      }
    }
    this.cache[key] = best;
    return best;
  }
}

/** Little-endian bytes, growing as needed. */
class Output {
  private bytes = new Uint8Array(1 << 16);
  length = 0;

  byte(value: number) {
    if (this.length === this.bytes.length) {
      const grown = new Uint8Array(this.bytes.length * 2);
      grown.set(this.bytes);
      this.bytes = grown;
    }
    this.bytes[this.length++] = value;
  }

  word(value: number) {
    this.byte(value & 0xff);
    this.byte((value >> 8) & 0xff);
  }

  all(values: ArrayLike<number>) {
    for (let index = 0; index < values.length; index++) this.byte(values[index]);
  }

  result() {
    return this.bytes.slice(0, this.length);
  }
}

/** 8-bit LZW into 255-byte sub-blocks, as omggif's writer does. */
function lzwEncode(indices: Uint8Array, out: Output) {
  const clear = 256;
  const end = 257;
  out.byte(8);
  const block = new Uint8Array(255);
  let filled = 0;
  let accumulator = 0;
  let bits = 0;
  let size = 9;
  const emit = (code: number) => {
    accumulator |= code << bits;
    bits += size;
    while (bits >= 8) {
      block[filled++] = accumulator & 0xff;
      accumulator >>>= 8;
      bits -= 8;
      if (filled === 255) {
        out.byte(255);
        out.all(block);
        filled = 0;
      }
    }
  };
  // Open addressing on (prefix << 8 | index); 4096 codes at most.
  const keys = new Int32Array(8192);
  const codes = new Int16Array(8192);
  const reset = () => keys.fill(-1);
  reset();
  let next = end + 1;
  emit(clear);
  let current = indices[0];
  for (let at = 1; at < indices.length; at++) {
    const index = indices[at];
    const key = (current << 8) | index;
    let slot = Math.imul(key, 0x9e3779b1) >>> 19;
    while (keys[slot] !== -1 && keys[slot] !== key) slot = (slot + 1) & 8191;
    if (keys[slot] === key) {
      current = codes[slot];
      continue;
    }
    emit(current);
    if (next === 4096) {
      emit(clear);
      reset();
      next = end + 1;
      size = 9;
    } else {
      if (next >= 1 << size) size++;
      keys[slot] = key;
      codes[slot] = next++;
    }
    current = index;
  }
  emit(current);
  emit(end);
  if (bits > 0) block[filled++] = accumulator & 0xff;
  if (filled) {
    out.byte(filled);
    out.all(block.subarray(0, filled));
  }
  out.byte(0);
}

interface Pending {
  rect: Rect;
  palette: Uint8Array;
  indices: Uint8Array;
  delay: number;
}

/**
 * Re-encodes `bytes` as an `edge` × `edge` GIF, every frame with its delay.
 * Each frame gets its own palette of the colours that changed; unchanged pixels are left
 * showing (transparent), unless the animation has transparency of its own, in
 * which case every frame is drawn whole on a cleared canvas.
 */
export function encodeShrunk(bytes: Uint8Array, edge: number): Uint8Array {
  const { width, height } = header(bytes);
  const side = Math.min(width, height);
  const left = Math.floor((width - side) / 2);
  const top = Math.floor((height - side) / 2);
  let transparency = false;
  decodeGif(bytes, (rgba) => {
    if (transparency) return;
    for (let y = top; y < top + side && !transparency; y++)
      for (let at = (y * width + left) * 4 + 3, x = 0; x < side; x++, at += 4)
        if (rgba[at] === 0) {
          transparency = true;
          break;
        }
  });
  const out = new Output();
  out.all([..."GIF89a"].map((char) => char.charCodeAt(0)));
  out.word(edge);
  out.word(edge);
  // No global palette: each frame carries its own.
  out.all([0x70, 0, 0]);
  out.all([0x21, 0xff, 11, ..."NETSCAPE2.0".split("").map((char) => char.charCodeAt(0)), 3, 1, 0, 0, 0]);
  const write = (frame: Pending) => {
    // Index 255 is transparent: real transparency, or "unchanged" between frames.
    out.all([0x21, 0xf9, 4, ((transparency ? 2 : 1) << 2) | 1]);
    out.word(Math.min(0xffff, frame.delay));
    out.all([255, 0, 0x2c]);
    out.word(frame.rect.x);
    out.word(frame.rect.y);
    out.word(frame.rect.w);
    out.word(frame.rect.h);
    out.byte(0x87);
    const table = new Uint8Array(768);
    table.set(frame.palette);
    out.all(table);
    lzwEncode(frame.indices, out);
  };
  const scaled = new Uint8ClampedArray(edge * edge * 4);
  let previous: Uint8ClampedArray | undefined;
  const histogram = new Histogram();
  let pending: Pending | undefined;
  decodeGif(bytes, (rgba, delay) => {
    if (pending) write(pending);
    squareScale(rgba, width, height, edge, scaled);
    // What this frame must draw: everything, or what differs from the last.
    let x0 = 0;
    let y0 = 0;
    let x1 = edge - 1;
    let y1 = edge - 1;
    const changed = (pixel: number) => {
      if (transparency || !previous) return true;
      const at = pixel * 4;
      return (
        scaled[at] !== previous[at] ||
        scaled[at + 1] !== previous[at + 1] ||
        scaled[at + 2] !== previous[at + 2] ||
        scaled[at + 3] !== previous[at + 3]
      );
    };
    if (!transparency && previous) {
      x0 = edge;
      y0 = edge;
      x1 = -1;
      y1 = -1;
      for (let pixel = 0; pixel < edge * edge; pixel++)
        if (changed(pixel)) {
          const x = pixel % edge;
          const y = (pixel - x) / edge;
          if (x < x0) x0 = x;
          if (x > x1) x1 = x;
          if (y < y0) y0 = y;
          if (y > y1) y1 = y;
        }
    }
    if (x1 < 0) {
      // Nothing changed: one transparent pixel still holds the frame's time.
      pending = { rect: { x: 0, y: 0, w: 1, h: 1 }, palette: new Uint8Array(3), indices: new Uint8Array([255]), delay };
      return;
    }
    const w = x1 - x0 + 1;
    const h = y1 - y0 + 1;
    for (let y = y0; y <= y1; y++)
      for (let x = x0; x <= x1; x++) {
        const at = (y * edge + x) * 4;
        if (scaled[at + 3] && changed(y * edge + x)) histogram.add(scaled[at], scaled[at + 1], scaled[at + 2]);
      }
    const palette = histogram.palette(255);
    const mapper = new Mapper(palette);
    const indices = new Uint8Array(w * h);
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const pixel = (y0 + y) * edge + x0 + x;
        const at = pixel * 4;
        indices[y * w + x] =
          scaled[at + 3] && changed(pixel) ? mapper.index(scaled[at], scaled[at + 1], scaled[at + 2]) : 255;
      }
    pending = { rect: { x: x0, y: y0, w, h }, palette, indices, delay };
    previous = scaled.slice();
  });
  if (pending) write(pending);
  out.byte(0x3b);
  return out.result();
}

/** `bytes` as a centered square of at most 512 px, every frame and its timing kept. */
export function shrinkGif(bytes: Uint8Array): { bytes: Uint8Array; edge: number } {
  const { width, height } = header(bytes);
  const edge = Math.min(SHRINK_EDGE, width, height);
  return { bytes: encodeShrunk(bytes, edge), edge };
}
