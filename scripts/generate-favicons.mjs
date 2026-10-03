// Raster fallbacks for the unchanged headphones avatar used as the site icon.
// Requires ImageMagick 7 and librsvg (rsvg-convert); node scripts/generate-favicons.mjs
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../apps/web/public/images/avatars/v3/1.svg', import.meta.url));
const output = fileURLToPath(new URL('../apps/web/public/icons/', import.meta.url));
mkdirSync(output, { recursive: true });
// librsvg preserves the avatar's circular clip, unlike ImageMagick's MSVG renderer.
const raster = execFileSync('rsvg-convert', ['--width', '1024', '--height', '1024', source]);
for (const size of [32, 180, 192]) {
  execFileSync('magick', [
    'png:-', '-resize', `${size}x${size}`,
    // iOS home-screen icons need an opaque square; favicons retain transparency.
    ...(size === 180 ? ['-background', '#0C0D0F', '-alpha', 'remove', '-alpha', 'off'] : []),
    '-depth', '8', '-strip', `${output}headphones-v3-${size}.png`,
  ], { input: raster });
}
console.log('Exported 32/192px favicon PNGs and an opaque 180px Apple touch icon from avatar 1.');
