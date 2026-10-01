// Run only when authoring a NEW collection: v1's pixels and mapping are immutable.
// Requires ImageMagick 7; no generation or image dependency at runtime/build time.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'caper-avatars-'));
const magick = (...args) => execFileSync('magick', args, { cwd: root });
try {
  const tiles = [];
  for (let index = 0; index < 800; index++) {
    const character = index % 100;
    const color = Math.floor(index / 100);
    const quadrant = character % 4;
    const source = character < 4 ? 'apps/web/public/images/demo-avatars.webp'
      : `assets/avatars/companions-${String(Math.floor(character / 4)).padStart(2, '0')}.webp`;
    const tile = join(scratch, `${index}.png`);
    magick(source, '-crop', `256x256+${(quadrant % 2) * 256}+${Math.floor(quadrant / 2) * 256}`, '+repage',
      '-resize', '64x64', '-modulate', `100,100,${100 + color * 25}`,
      '(', '+clone', '-threshold', '-1', '-fill', 'black', '-draw', 'rectangle 0,0 63,63',
      '-fill', 'white', '-draw', 'circle 31.5,31.5 31.5,0', ')', '-alpha', 'off', '-compose', 'CopyOpacity', '-composite', tile);
    tiles.push(tile);
  }
  magick('montage', ...tiles, '-tile', '32x25', '-geometry', '64x64+0+0', '-background', 'none',
    '-strip', 'apps/web/public/images/avatars/capers-v1.png');
  magick('apps/web/public/images/avatars/capers-v1.png', '-quality', '88', 'apps/web/public/images/avatars/capers-v1.webp');
  console.log('Generated 800 avatars: 100 characters × 8 hues (2048×1600 atlas).');
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
