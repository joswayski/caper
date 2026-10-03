// Production web head/assets and a labelled icon inspection surface, not OS chrome.
// node scripts/test-favicons.mjs http://localhost:31095 [screenshot-directory]
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

const origin = new URL(process.argv[2]);
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname));
const artifacts = process.argv[3] && resolve(process.argv[3]);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const expected = [
  { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/icons/headphones-v3-32.png' },
  { rel: 'icon', type: 'image/png', sizes: '192x192', href: '/icons/headphones-v3-192.png' },
  { rel: 'icon', type: 'image/svg+xml', sizes: 'any', href: '/images/avatars/v3/1.svg' },
  { rel: 'apple-touch-icon', type: null, sizes: '180x180', href: '/icons/headphones-v3-180.png' },
];
// These declarations must be in server HTML, not inserted only after hydration.
for (const route of ['/', '/login']) {
  const response = await fetch(new URL(route, origin));
  assert.equal(response.status, 200);
  const html = await response.text();
  const links = [...html.matchAll(/<link\b[^>]*>/g)].map(([tag]) => Object.fromEntries(
    ['rel', 'type', 'sizes', 'href'].map(key => [key, tag.match(new RegExp(`${key}="([^"]*)"`))?.[1] ?? null]),
  )).filter(link => ['icon', 'apple-touch-icon'].includes(link.rel));
  assert.deepEqual(links, expected, `Server icon declarations on ${route}`);
}
for (const { href, type, sizes } of expected) {
  const response = await fetch(new URL(href, origin));
  assert.equal(response.status, 200, href);
  assert.match(response.headers.get('content-type'), new RegExp(`^${(type ?? 'image/png').replace('+', '\\+')}`));
  if (sizes === 'any') continue;
  const png = Buffer.from(await response.arrayBuffer());
  assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
  assert.equal(png.readUInt32BE(16), Number(sizes.split('x')[0]));
  assert.equal(png.readUInt32BE(20), Number(sizes.split('x')[1]));
}

const session = `favicons-${process.pid}`;
const browser = (...args) => execFileSync('agent-browser', ['--session', session, ...args], { encoding: 'utf8', timeout: 40000 });
const evaluate = code => JSON.parse(browser('eval', code));
try {
  for (const width of [1280, 390]) {
    browser('open', `${origin}`);
    browser('set', 'viewport', String(width), '844', '2');
    assert.deepEqual(evaluate(`[...document.querySelectorAll('link[rel="icon"],link[rel="apple-touch-icon"]')].map(e => Object.fromEntries(['rel','type','sizes','href'].map(key => [key,e.getAttribute(key)])))`), expected);
    assert.equal(evaluate(`(async () => {
      for (const link of document.querySelectorAll('link[rel="icon"],link[rel="apple-touch-icon"]')) {
        const image = new Image(); image.src = link.href; await image.decode();
        const canvas = document.createElement('canvas'); canvas.width = canvas.height = 32;
        const ctx = canvas.getContext('2d'); ctx.drawImage(image,0,0,32,32);
        const corner = [...ctx.getImageData(0,0,1,1).data];
        const apple = link.rel === 'apple-touch-icon';
        if (apple ? corner.join() !== '12,13,15,255' : corner[3] !== 0) throw Error('Wrong corner alpha/background: '+link.href);
        const face = [...ctx.getImageData(16,12,1,1).data];
        if (face.join() !== '96,120,65,255') throw Error('Wrong avatar face: '+link.href+' '+face);
      }
      return true;
    })()`), true);
  }
  if (artifacts) {
    browser('eval', `(() => {
      const stage = document.createElement('section'); stage.id = 'favicon-review';
      stage.innerHTML = '<style>#favicon-review{position:fixed;inset:0;z-index:99999;overflow:auto;background:#0c0d0f;color:#f3f4f5;padding:24px;font:14px Satoshi,sans-serif}#favicon-review h1{font-size:22px;margin:0 0 8px}#favicon-review p{margin:0 0 20px;color:#adb2b5}.icon-panels{display:grid;grid-template-columns:1fr 1fr;gap:16px}.icon-panel{padding:20px;border:1px solid #34383b;border-radius:8px}.icon-panel h2{font-size:14px;margin:0 0 16px}.icon-row{display:flex;align-items:center;gap:24px;margin:16px 0}.icon-label{width:48px}.light-panel{background:#f3f4f5;color:#151719}.large-icons{display:flex;gap:32px;margin-top:24px}.large-icons figure{margin:0}.large-icons figcaption{margin-bottom:12px;color:#adb2b5}.large-icons img{width:180px;height:180px}@media(max-width:600px){.icon-panels{grid-template-columns:1fr}.large-icons{gap:16px}.large-icons img{width:150px;height:150px}}</style><h1>Caper · headphones avatar icon</h1><p>Rendered asset inspection · not a browser or home-screen screenshot</p><div class="icon-panels"></div><div class="large-icons"><figure><figcaption>Scalable SVG · enlarged</figcaption><img src="/images/avatars/v3/1.svg" alt="Green Caper wearing headphones"></figure><figure><figcaption>iOS home-screen PNG</figcaption><img src="/icons/headphones-v3-180.png" alt="Opaque Apple touch icon"></figure></div>';
      const apple = stage.querySelector('img[src$="180.png"]');
      // Show this 180px PNG at native pixel density, and expose its opaque corners.
      apple.style.cssText = 'width:90px;height:90px;box-sizing:content-box;border:12px solid #f3f4f5';
      for (const light of [false,true]) {
        const panel = document.createElement('div'); panel.className = 'icon-panel'+(light?' light-panel':'');
        panel.innerHTML = '<h2>'+(light?'Light':'Dark')+' background · 16 / 32 / 64px</h2>';
        for (const [label,src] of [['Before','/caper-face.svg?v=3'],['SVG','/images/avatars/v3/1.svg'],['PNG','/icons/headphones-v3-32.png']]) {
          panel.innerHTML += '<div class="icon-row"><span class="icon-label">'+label+'</span>'+[16,32,64].map(size=>'<img src="'+(label==='PNG' && size>16?'/icons/headphones-v3-192.png':src)+'" width="'+size+'" height="'+size+'" alt="'+label+' at '+size+'px">').join('')+'</div>';
        }
        stage.querySelector('.icon-panels').append(panel);
      }
      document.body.append(stage);
      return Promise.all([...stage.querySelectorAll('img')].map(image=>image.decode()));
    })()`);
    for (const [width, height, name] of [[880,720,'favicons-desktop.png'],[390,1100,'favicons-narrow.png']]) {
      browser('set', 'viewport', String(width), String(height), '2');
      browser('eval', 'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
      browser('screenshot', join(artifacts, name));
    }
  }
  console.log('Favicon checks passed: SSR links on home/login, HTTP MIME/dimensions, desktop/narrow DOM, SVG/PNG decode, transparent favicons and opaque Apple icon.');
} finally {
  browser('close');
}
