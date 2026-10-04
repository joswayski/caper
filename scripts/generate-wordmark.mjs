// Keep the original lettering geometry; clients compose the daily character in its dot slot.
// node scripts/generate-wordmark.mjs [--check]
import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = new URL('../', import.meta.url);
const original = readFileSync(new URL('apps/web/public/caper-wordmark.svg', root), 'utf8');
const lettering = original.match(/<path fill="#f3f4f5" fill-rule="evenodd" d="([^"]+)"\/>/);
assert.ok(lettering, 'Canonical wordmark must contain its original lettering path');
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="20 17 1042 276">\n  ${lettering[0]}\n</svg>\n`;
const apple = 'apps/native/apple/Resources/CaperIcons.xcassets/CaperWordmarkLetters.imageset';
const files = new Map([
  ['apps/web/public/caper-wordmark-letters.svg', svg],
  [`${apple}/wordmark.svg`, svg],
  [`${apple}/Contents.json`, JSON.stringify({
    images: [{ filename: 'wordmark.svg', idiom: 'universal' }],
    info: { author: 'xcode', version: 1 },
    properties: { 'preserves-vector-representation': true, 'template-rendering-intent': 'original' },
  }, null, 2) + '\n'],
  ['apps/native/android/app/src/main/res/drawable/caper_wordmark_letters.xml',
    '<?xml version="1.0" encoding="utf-8"?>\n'
    + '<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="1042dp" android:height="276dp" android:viewportWidth="1042" android:viewportHeight="276">\n'
    + `  <group android:translateX="-20" android:translateY="-17"><path android:pathData="${lettering[1]}" android:fillColor="#F3F4F5" android:fillType="evenOdd"/></group>\n`
    + '</vector>\n'],
]);
for (const [relative, value] of files) {
  const path = fileURLToPath(new URL(relative, root));
  if (process.argv.includes('--check')) assert.equal(readFileSync(path, 'utf8'), value, `Wordmark drift: ${relative}`);
  else { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, value); }
}
console.log(`${process.argv.includes('--check') ? 'Verified' : 'Exported'} dot-free wordmark lettering for web, Rust desktop, Apple and Android.`);
