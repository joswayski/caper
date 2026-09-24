// Commit the same Lucide vectors used by the web client. No JS or SVG network
// dependency is needed at native runtime. Run after changing web icon versions.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
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
mkdirSync(output, { recursive: true });
const files = Object.entries(names).map(([file, name]) => [`${file}.svg`, `${renderToStaticMarkup(createElement(icons[name], { color: '#ffffff', size: 24 }))}\n`]);
files.push(['LICENSE', readFileSync('node_modules/lucide-react/LICENSE', 'utf8')]);
for (const [file, content] of files) {
  if (process.argv.includes('--check')) {
    if (readFileSync(`${output}/${file}`, 'utf8') !== content) throw Error(`Native icon drift: ${file}`);
  } else {
    writeFileSync(`${output}/${file}`, content);
  }
}
console.log(`${process.argv.includes('--check') ? 'Verified' : 'Exported'} ${Object.keys(names).length} web Lucide icons for native desktop.`);
