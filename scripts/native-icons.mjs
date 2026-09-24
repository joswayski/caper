// Commit the same Lucide vectors used by the web client. No JS or SVG network
// dependency is needed at native runtime. Run after changing web icon versions.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import * as icons from 'lucide-react';

const names = {
  'chevron-down': 'ChevronDown', 'chevron-right': 'ChevronRight',
  x: 'X', plus: 'Plus', ellipsis: 'Ellipsis',
  settings: 'Settings', hash: 'Hash', lock: 'Lock', users: 'Users',
  speech: 'Speech', mic: 'Mic', 'mic-off': 'MicOff',
  headphones: 'Headphones', 'volume-x': 'VolumeX', menu: 'Menu',
};
const output = resolve('apps/native/desktop/resources/icons');
const apple = resolve('apps/native/apple/Resources/CaperIcons.xcassets');
const info = { author: 'xcode', version: 1 };
const json = (value) => `${JSON.stringify(value, null, 2)}\n`;
const files = [[`${apple}/Contents.json`, json({ info })]];
for (const [file, name] of Object.entries(names)) {
  const svg = `${renderToStaticMarkup(createElement(icons[name], { color: '#ffffff', size: 24 }))}\n`;
  const image = `${apple}/caper-${file}.imageset`;
  files.push([`${output}/${file}.svg`, svg], [`${image}/${file}.svg`, svg], [`${image}/Contents.json`, json({
    images: [{ filename: `${file}.svg`, idiom: 'universal' }], info,
    properties: { 'preserves-vector-representation': true, 'template-rendering-intent': 'template' },
  })]);
}
const license = readFileSync('node_modules/lucide-react/LICENSE', 'utf8');
files.push([`${output}/LICENSE`, license], [resolve('apps/native/apple/Resources/Lucide-LICENSE.txt'), license]);
for (const [file, content] of files) {
  if (process.argv.includes('--check')) {
    if (readFileSync(file, 'utf8') !== content) throw Error(`Native icon drift: ${file}`);
  } else {
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, content);
  }
}
console.log(`${process.argv.includes('--check') ? 'Verified' : 'Exported'} ${Object.keys(names).length} web Lucide icons for Rust and Apple native clients.`);
