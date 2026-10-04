// Original mascot for static web/desktop identities and pre-JavaScript fallbacks.
// Requires ImageMagick 7 and librsvg (rsvg-convert); node scripts/generate-favicons.mjs
// Use --check to detect stale assets without writing them.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../apps/web/public/caper-face.svg', import.meta.url));
const output = fileURLToPath(new URL('../apps/web/public/icons/', import.meta.url));
const desktop = fileURLToPath(new URL('../apps/native/desktop/resources/', import.meta.url));
const files = new Map();
const raster = execFileSync('rsvg-convert', ['--width', '1024', '--height', '1024', source]);
for (const size of [32, 180, 192, 512]) {
  const png = execFileSync('magick', [
    'png:-', '-resize', `${size}x${size}`,
    // Installed icons are opaque; the small browser fallback retains transparency.
    ...(size !== 32 ? ['-background', '#0C0D0F', '-alpha', 'remove', '-alpha', 'off'] : []),
    '-depth', '8', '-strip', 'png:-',
  ], { input: raster });
  files.set(`${output}caper-main-v3-${size}.png`, png);
}

// Windows Search and Explorer choose different ICO frames at different DPIs.
// Render every frame from the main artwork, not an intermediate Apple icon.
// Store explicit 32-bit RGBA PNGs to preserve the transparent smile and accent.
const sizes = [16, 24, 32, 48, 64, 128, 256];
const frames = sizes.map(size => execFileSync('magick', [
  'png:-', '-resize', `${size}x${size}`, '-depth', '8', '-strip', 'PNG32:-',
], { input: raster }));
files.set(`${desktop}caper-icon.png`, frames[sizes.indexOf(128)]);

// ICO header + directory, followed by PNG payloads (supported since Vista).
const directory = Buffer.alloc(6 + 16 * sizes.length);
directory.writeUInt16LE(1, 2); // Image type: icon.
directory.writeUInt16LE(sizes.length, 4);
let offset = directory.length;
for (const [index, size] of sizes.entries()) {
  const entry = 6 + 16 * index;
  directory[entry] = directory[entry + 1] = size === 256 ? 0 : size;
  directory.writeUInt16LE(1, entry + 4); // Color planes.
  directory.writeUInt16LE(32, entry + 6); // Bits per pixel.
  directory.writeUInt32LE(frames[index].length, entry + 8);
  directory.writeUInt32LE(offset, entry + 12);
  offset += frames[index].length;
}
files.set(`${desktop}caper.ico`, Buffer.concat([directory, ...frames]));

for (const [file, bytes] of files) {
  if (process.argv.includes('--check')) {
    assert.ok(readFileSync(file).equals(bytes), `Static Caper icon drift: ${file}`);
  } else {
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, bytes);
  }
}
console.log(`${process.argv.includes('--check') ? 'Verified' : 'Exported'} static web PNGs and desktop PNG/ICO from the main mascot SVG.`);
