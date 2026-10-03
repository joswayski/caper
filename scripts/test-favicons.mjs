// Clock-controlled real browser checks; inspection captures are not browser/OS chrome.
// node scripts/test-favicons.mjs http://localhost:31095 [screenshot-directory]
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const origin = new URL(process.argv[2]);
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname));
const artifacts = process.argv[3] && resolve(process.argv[3]);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const expected = [
  { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/icons/caper-main-v3-32.png' },
  { rel: 'icon', type: 'image/png', sizes: '192x192', href: '/icons/caper-main-v3-192.png' },
  { rel: 'icon', type: 'image/svg+xml', sizes: 'any', href: '/caper-face.svg?v=3' },
  { rel: 'apple-touch-icon', type: null, sizes: '180x180', href: '/icons/caper-main-v3-180.png' },
];
for (const route of ['/', '/login']) {
  const response = await fetch(new URL(route, origin));
  assert.equal(response.status, 200);
  const html = await response.text();
  const links = [...html.matchAll(/<link\b[^>]*>/g)].map(([tag]) => Object.fromEntries(
    ['rel', 'type', 'sizes', 'href'].map(key => [key, tag.match(new RegExp(`${key}="([^"]*)"`))?.[1] ?? null]),
  )).filter(link => ['icon', 'apple-touch-icon'].includes(link.rel));
  assert.deepEqual(links, expected, `Original mascot in server HTML on ${route}`);
}
const manifestResponse = await fetch(new URL('/site.webmanifest', origin));
assert.equal(manifestResponse.status, 200);
const manifest = await manifestResponse.json();
assert.equal(manifest.display, 'browser');
assert.deepEqual(manifest.icons, [192, 512].map(size => ({ src: `/icons/caper-main-v3-${size}.png`, sizes: `${size}x${size}`, type: 'image/png' })));
for (const { href, type, sizes } of [...expected, { href: '/icons/caper-main-v3-512.png', type: 'image/png', sizes: '512x512' }]) {
  const response = await fetch(new URL(href, origin));
  assert.equal(response.status, 200, href);
  assert.match(response.headers.get('content-type'), new RegExp(`^${(type ?? 'image/png').replace('+', '\\+')}`));
  if (sizes === 'any') continue;
  const png = Buffer.from(await response.arrayBuffer());
  assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
  assert.equal(png.readUInt32BE(16), Number(sizes.split('x')[0]));
  assert.equal(png.readUInt32BE(20), Number(sizes.split('x')[1]));
}

const scratch = mkdtempSync(join(tmpdir(), 'caper-icons-'));
const init = join(scratch, 'clock.js');
function fixture() {
  if (location.protocol === 'about:') return;
  window.__iconNow = Number(new URL(location.href).searchParams.get('icon-now') || Date.UTC(2026, 9, 3, 23, 59, 59, 999));
  Date.now = () => window.__iconNow;
  Math.random = () => 0;
  if (!localStorage.getItem('caper.daily-icon.v1')) localStorage.setItem('caper.daily-icon.v1', JSON.stringify({ day: Math.floor(window.__iconNow / 86400000), index: 799 }));
  const interval = window.setInterval.bind(window);
  window.__iconTimers = [];
  window.setInterval = (callback, ms, ...args) => {
    if (ms === 60000) window.__iconTimers.push(() => callback(...args));
    return interval(callback, ms, ...args);
  };
  if (location.search.includes('installed-icon-test')) Object.defineProperty(navigator, 'standalone', { value: true });
}
writeFileSync(init, `(${fixture.toString()})();`);
const session = `favicons-${process.pid}`;
const browser = (...args) => execFileSync('agent-browser', ['--session', session, '--init-script', init, ...args], { encoding: 'utf8', timeout: 40000 });
const evaluate = code => JSON.parse(browser('eval', code));
const waitForIcon = (index, previousPNG = '') => browser('wait', '--fn', `document.querySelector('#caper-favicon-svg')?.getAttribute('href') === '/images/avatars/v3/${index}.svg' && document.querySelector('#caper-favicon-32')?.href.startsWith('data:image/png') && document.querySelector('#caper-favicon-32')?.href !== ${JSON.stringify(previousPNG)}`);
const record = () => evaluate(`Object.fromEntries(['svg','32','192'].map(key => [key,document.querySelector('#caper-favicon-'+key).getAttribute('href')]))`);
try {
  browser('open', `${origin}`);
  browser('set', 'viewport', '1280', '844', '2');
  waitForIcon(799);
  const today = record();
  browser('reload');
  waitForIcon(799);
  assert.equal(record().svg, today.svg, 'Same-day reload keeps the random avatar');
  browser('snapshot', '-i');
  browser('find', 'first', 'a[href="/login"]', 'click');
  browser('wait', '--url', '**/login');
  waitForIcon(799);
  browser('eval', `window.__iconNow += 1; window.dispatchEvent(new Event('focus'));`);
  waitForIcon(0, today['32']);
  const tomorrow = record();
  assert.equal(evaluate(`JSON.parse(localStorage.getItem('caper.daily-icon.v1')).index`), 0);
  browser('eval', `window.__iconNow += 86400000; window.__iconTimers.forEach(tick => tick());`);
  waitForIcon(1, tomorrow['32']);
  assert.equal(evaluate(`(async () => {
    const source = new Image(); source.src = document.querySelector('#caper-favicon-svg').href; await source.decode();
    for (const size of [32,192]) {
      const image = new Image(); image.src = document.querySelector('#caper-favicon-'+size).href; await image.decode();
      if (image.width !== size || image.height !== size) throw Error('Wrong PNG dimensions');
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 32;
      const ctx = canvas.getContext('2d'); ctx.drawImage(image,0,0,32,32);
      if (ctx.getImageData(0,0,1,1).data[3] !== 0 || ctx.getImageData(16,12,1,1).data[3] !== 255) throw Error('Wrong avatar alpha');
      if (size === 32) {
        const pixel = [...ctx.getImageData(16,12,1,1).data]; ctx.clearRect(0,0,32,32); ctx.drawImage(source,0,0,32,32);
        if (pixel.join() !== [...ctx.getImageData(16,12,1,1).data].join()) throw Error('PNG and SVG depict different avatars');
      }
    }
    const apple = document.querySelector('link[rel="apple-touch-icon"]');
    if (apple.getAttribute('href') !== '/icons/caper-main-v3-180.png') throw Error('Installed icon rotated');
    const image = new Image(); image.src = apple.href; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 32;
    const ctx = canvas.getContext('2d'); ctx.drawImage(image,0,0,32,32);
    if ([...ctx.getImageData(0,0,1,1).data].join() !== '12,13,15,255') throw Error('Touch icon must be opaque');
    return true;
  })()`), true);
  browser('set', 'viewport', '390', '844', '2');
  assert.equal(record().svg, '/images/avatars/v3/1.svg');
  if (artifacts) {
    browser('eval', `(() => {
      const stage = document.createElement('section'); stage.id = 'favicon-review';
      stage.innerHTML = '<style>#favicon-review{position:fixed;inset:0;z-index:99999;overflow:auto;background:#0c0d0f;color:#f3f4f5;padding:24px;font:14px Satoshi,sans-serif}#favicon-review h1{font-size:22px;margin:0 0 8px}#favicon-review p{margin:0 0 20px;color:#adb2b5}.icon-panels{display:grid;grid-template-columns:1fr 1fr;gap:16px}.icon-panel{padding:16px;border:1px solid #34383b;border-radius:8px}.icon-panel h2{font-size:14px;margin:0 0 16px}.icon-row{display:flex;align-items:center;gap:16px;margin:16px 0;padding:12px}.icon-label{width:32px}.light{background:#f3f4f5;color:#151719}.fixed-icon{margin-top:24px}.fixed-icon img{width:90px;height:90px;box-sizing:content-box;border:12px solid #f3f4f5}@media(max-width:600px){.icon-panels{grid-template-columns:1fr}}</style><h1>Caper · daily rotating avatars</h1><p>Clock-controlled browser test · rendered assets, not OS chrome</p><div class="icon-panels"></div><div class="fixed-icon"><p>Unsupported / installed surfaces · original mascot stays fixed</p><img src="/icons/caper-main-v3-180.png" alt="Original green Caper icon"></div>';
      for (const [i,sample] of ${JSON.stringify([today, tomorrow])}.entries()) {
        const panel = document.createElement('div'); panel.className = 'icon-panel';
        panel.innerHTML = '<h2>Day '+(i+1)+' · avatar '+(i===0?'799':'0')+'</h2>';
        for (const light of [false,true]) for (const format of ['SVG','PNG']) {
          panel.innerHTML += '<div class="icon-row '+(light?'light':'')+'"><span class="icon-label">'+format+'</span>'+[16,32,64].map(size=>'<img src="'+(format==='SVG'?sample.svg:sample['192'])+'" width="'+size+'" height="'+size+'" alt="Day '+(i+1)+' '+format+' '+size+'px">').join('')+'</div>';
        }
        stage.querySelector('.icon-panels').append(panel);
      }
      document.body.append(stage);
      return Promise.all([...stage.querySelectorAll('img')].map(image=>image.decode()));
    })()`);
    for (const [width,height,name] of [[880,840,'favicons-rotating-desktop.png'],[390,1480,'favicons-rotating-narrow.png']]) {
      browser('set', 'viewport', String(width), String(height), '2');
      browser('eval', 'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
      browser('screenshot', join(artifacts, name));
    }
  }
  const clock = evaluate('window.__iconNow');
  browser('tab', 'new', `${origin}?icon-now=${clock}`);
  waitForIcon(1);
  browser('tab', 't1');
  browser('eval', 'Math.random = () => 0.5;');
  browser('tab', 't2');
  // Mock another tab's saved assignment; this exercises real cross-tab storage events.
  browser('eval', 'localStorage.setItem("caper.daily-icon.v1", JSON.stringify({day:Math.floor(Date.now()/86400000),index:77})); window.dispatchEvent(new Event("focus"));');
  waitForIcon(77);
  browser('tab', 't1');
  browser('eval', 'window.dispatchEvent(new Event("focus"));');
  waitForIcon(77);
  assert.equal(record().svg, '/images/avatars/v3/77.svg', 'Other tabs adopt the saved daily icon instead of rerolling');
  browser('open', `${origin}?installed-icon-test=1`);
  browser('wait', '1000');
  assert.equal(evaluate(`document.querySelector('#caper-favicon-svg').getAttribute('href')`), '/caper-face.svg?v=3', 'Explicitly mocked standalone launch keeps original identity');
  console.log('Favicon checks passed: SSR/manifest original mascot; same-day reload + SPA stability; UTC focus + timer rotation; cross-tab persistence; SVG/PNG agreement; installed-icon fallback; desktop/narrow layouts.');
} finally {
  browser('close');
  rmSync(scratch, { recursive: true, force: true });
}
