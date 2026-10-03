// Original mascot for non-rotating installed shortcuts and pre-JavaScript fallbacks.
// Requires ImageMagick 7 and librsvg (rsvg-convert); node scripts/generate-favicons.mjs
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../apps/web/public/caper-face.svg', import.meta.url));
const output = fileURLToPath(new URL('../apps/web/public/icons/', import.meta.url));
mkdirSync(output, { recursive: true });
const raster = execFileSync('rsvg-convert', ['--width', '1024', '--height', '1024', source]);
for (const size of [32, 180, 192, 512]) {
  execFileSync('magick', [
    'png:-', '-resize', `${size}x${size}`,
    // Installed icons are opaque; the small browser fallback retains transparency.
    ...(size !== 32 ? ['-background', '#0C0D0F', '-alpha', 'remove', '-alpha', 'off'] : []),
    '-depth', '8', '-strip', `${output}caper-main-v3-${size}.png`,
  ], { input: raster });
}
console.log('Exported original mascot PNGs at 32/180/192/512px; installed shortcuts retain Caper identity.');
