// Regenerate the native artwork from the same locked packages as the web picker.
// Requires npm ci and librsvg (rsvg-convert). Native builds consume the committed output.
import { execFile } from "node:child_process";
import { cp, mkdir, readdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { availableParallelism } from "node:os";
import { fileURLToPath } from "node:url";
import { basename, join } from "node:path";
import { promisify } from "node:util";
import { deflateSync, inflateSync } from "node:zlib";
import { emojiAsset, emojiNames } from "../apps/web/src/chat/emoji.ts";
import "../apps/web/scripts/prepare-emoji.mjs";
import labels from "../apps/web/src/chat/emoji-labels.json" with { type: "json" };

const execFileAsync = promisify(execFile);
const CELL = 128;
const GRID = 16;
const SHEET = CELL * GRID;
const PER_SHEET = GRID * GRID;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const crcTable = new Uint32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  crcTable[n] = c >>> 0;
}

function crc32(buffer) {
  let c = 0xffffffff;
  for (let i = 0; i < buffer.length; i++) c = crcTable[(c ^ buffer[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const chunk = Buffer.allocUnsafe(12 + data.length);
  chunk.writeUInt32BE(data.length, 0);
  chunk.write(type, 4, 4, "ascii");
  data.copy(chunk, 8);
  chunk.writeUInt32BE(crc32(chunk.subarray(4, 8 + data.length)), 8 + data.length);
  return chunk;
}

function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  if (pa <= pb && pa <= pc) return a;
  return pb <= pc ? b : c;
}

/** Decode an 8-bit non-interlaced RGBA or RGB PNG emitted by rsvg-convert. */
function decodePngRgba(buffer) {
  if (!buffer.subarray(0, 8).equals(PNG_SIGNATURE)) throw new Error("Invalid PNG signature.");
  let width = 0;
  let height = 0;
  let bitDepth = 0;
  let colorType = 0;
  const idat = [];
  for (let offset = 8; offset < buffer.length;) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.toString("ascii", offset + 4, offset + 8);
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    offset += 12 + length;
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      if (bitDepth !== 8 || (colorType !== 6 && colorType !== 2) || data[12] !== 0) {
        throw new Error(`Unsupported PNG format: depth=${bitDepth}, colorType=${colorType}`);
      }
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }
  }
  const bpp = colorType === 6 ? 4 : 3;
  const stride = width * bpp;
  const raw = inflateSync(Buffer.concat(idat));
  const unpacked = Buffer.allocUnsafe(height * stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const src = y * (stride + 1) + 1;
    const dst = y * stride;
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? unpacked[dst + x - bpp] : 0;
      const b = y > 0 ? unpacked[dst - stride + x] : 0;
      const c = y > 0 && x >= bpp ? unpacked[dst - stride + x - bpp] : 0;
      const val = raw[src + x];
      if (filter === 0) unpacked[dst + x] = val;
      else if (filter === 1) unpacked[dst + x] = (val + a) & 0xff;
      else if (filter === 2) unpacked[dst + x] = (val + b) & 0xff;
      else if (filter === 3) unpacked[dst + x] = (val + ((a + b) >> 1)) & 0xff;
      else if (filter === 4) unpacked[dst + x] = (val + paeth(a, b, c)) & 0xff;
      else throw new Error(`Unsupported PNG filter ${filter}`);
    }
  }
  if (colorType === 6) return { width, height, rgba: unpacked };
  const rgba = Buffer.allocUnsafe(width * height * 4);
  for (let i = 0, j = 0; i < unpacked.length; i += 3, j += 4) {
    rgba[j] = unpacked[i];
    rgba[j + 1] = unpacked[i + 1];
    rgba[j + 2] = unpacked[i + 2];
    rgba[j + 3] = 255;
  }
  return { width, height, rgba };
}

/** Encode an 8-bit RGBA PNG (color type 6) using per-row adaptive filtering. */
function encodePngRgba(width, height, rgba) {
  const stride = width * 4;
  const filtered = Buffer.allocUnsafe(height * (stride + 1));
  const candidate = Buffer.allocUnsafe(stride);
  for (let y = 0; y < height; y++) {
    const row = y * stride;
    const prev = row - stride;
    let bestFilter = 0;
    let bestScore = Infinity;
    for (let filter = 0; filter <= 4; filter++) {
      let score = 0;
      for (let x = 0; x < stride; x++) {
        const cur = rgba[row + x];
        const a = x >= 4 ? rgba[row + x - 4] : 0;
        const b = y > 0 ? rgba[prev + x] : 0;
        const c = y > 0 && x >= 4 ? rgba[prev + x - 4] : 0;
        let predicted = 0;
        if (filter === 1) predicted = a;
        else if (filter === 2) predicted = b;
        else if (filter === 3) predicted = (a + b) >> 1;
        else if (filter === 4) predicted = paeth(a, b, c);
        const diff = (cur - predicted) & 0xff;
        candidate[x] = diff;
        score += diff < 128 ? diff : 256 - diff;
      }
      if (score < bestScore) {
        bestScore = score;
        bestFilter = filter;
        candidate.copy(filtered, y * (stride + 1) + 1);
      }
    }
    filtered[y * (stride + 1)] = bestFilter;
  }
  const ihdr = Buffer.allocUnsafe(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // 8 bits per channel
  ihdr[9] = 6; // RGBA (PNG32)
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  const idat = deflateSync(filtered, { level: 9 });
  return Buffer.concat([
    PNG_SIGNATURE,
    pngChunk("IHDR", ihdr),
    pngChunk("IDAT", idat),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

async function rasterizeSvg(svgPath) {
  const { stdout } = await execFileAsync(
    "rsvg-convert",
    ["-w", String(CELL), "-h", String(CELL), "--keep-aspect-ratio", svgPath],
    { encoding: "buffer", maxBuffer: 4 * 1024 * 1024 },
  );
  return decodePngRgba(stdout);
}

async function mapPool(items, concurrency, worker) {
  const results = Array.from({ length: items.length });
  let cursor = 0;
  await Promise.all(
    Array.from({ length: Math.min(concurrency, items.length) }, async () => {
      while (cursor < items.length) {
        const index = cursor++;
        results[index] = await worker(items[index], index);
      }
    }),
  );
  return results;
}

const require = createRequire(import.meta.url);
const source = fileURLToPath(new URL("../apps/web/public/emoji/twemoji-15/", import.meta.url));
const target = fileURLToPath(new URL("../shared/emoji/", import.meta.url));
await mkdir(target, { recursive: true });
const data = require("emoji-picker-react/dist/data/emojis-en").default;
const offered = new Map();
for (const [category, entries] of Object.entries(data.emojis)) {
  for (const entry of entries.filter((item) => Number(item.a) <= 15)) {
    const id = basename(emojiAsset(entry.u), ".svg");
    const names = emojiNames(entry.n, labels[entry.u]);
    offered.set(id, {
      emoji: String.fromCodePoint(...entry.u.split("-").map((point) => parseInt(point, 16))),
      name: names.at(-1),
      keywords: names.join(" "),
      category,
    });
  }
}
const files = (await readdir(source)).filter((name) => name.endsWith(".svg")).sort();
const entries = files.map((file, index) => {
  const id = basename(file, ".svg");
  return {
    // Artwork IDs omit some selectors; composer insertion must preserve them.
    id,
    emoji: offered.get(id)?.emoji ?? String.fromCodePoint(...id.split("-").map((point) => parseInt(point, 16))),
    name: offered.get(id)?.name ?? id,
    keywords: offered.get(id)?.keywords ?? "",
    category: offered.get(id)?.category ?? "",
    selectable: offered.has(id),
    sheet: Math.floor(index / PER_SHEET),
    x: (index % GRID) * CELL,
    y: Math.floor((index % PER_SHEET) / GRID) * CELL,
  };
});
// Metadata-only updates need not rerasterize unchanged licensed artwork.
const concurrency = Math.max(2, availableParallelism());
for (let index = 0; !process.argv.includes("--catalog-only") && index < files.length; index += PER_SHEET) {
  const batch = files.slice(index, index + PER_SHEET);
  const tiles = await mapPool(batch, concurrency, (file) => rasterizeSvg(join(source, file)));
  const sheet = Buffer.alloc(SHEET * SHEET * 4);
  for (let i = 0; i < tiles.length; i++) {
    const tile = tiles[i];
    if (tile.width !== CELL || tile.height !== CELL) {
      throw new Error(`Expected ${CELL}x${CELL} tile for ${batch[i]}, got ${tile.width}x${tile.height}`);
    }
    const cellX = (i % GRID) * CELL;
    const cellY = Math.floor(i / GRID) * CELL;
    for (let row = 0; row < CELL; row++) {
      const srcOffset = row * CELL * 4;
      const dstOffset = ((cellY + row) * SHEET + cellX) * 4;
      tile.rgba.copy(sheet, dstOffset, srcOffset, srcOffset + CELL * 4);
    }
  }
  const output = join(target, `sheet-${index / PER_SHEET}.png`);
  await writeFile(output, encodePngRgba(SHEET, SHEET, sheet));
}
await writeFile(join(target, "catalog.json"), JSON.stringify(entries) + "\n");
// SwiftPM bundles resources inside its target directory. Keep that generated
// copy (including attribution/licenses) identical to Android/desktop's source.
await cp(target, fileURLToPath(new URL("../apps/native/apple/Sources/CaperCore/EmojiAssets/", import.meta.url)), {
  recursive: true,
});
console.log(
  `Generated ${entries.length} image entries, ${offered.size} picker choices, ${Math.ceil(files.length / PER_SHEET)} sheets (${CELL}px cells).`,
);
