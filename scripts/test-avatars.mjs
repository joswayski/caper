// Real account UI with explicitly mocked API/gateway responses. No signup or SFU calls.
// node scripts/test-avatars.mjs http://localhost:30701 [screenshot-directory]
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const origin = new URL(process.argv[2] ?? 'http://localhost:30701');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname));
const artifacts = process.argv[3] && resolve(process.argv[3]);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), 'avatar-test-'));
const init = join(scratch, 'fixture.js');
function fixture() {
  if (location.protocol === 'about:') return;
  // Missing fields represent an old/incomplete response, not an existing account
  // after migration. Keep that compatibility case separate from normal screenshots.
  const missingAvatar = new URLSearchParams(location.search).has('missing-avatar-test');
  const account = { id: 'owner1234567', username: 'fixture_owner', displayName: 'Alex', avatarId: 0 };
  const members = [account, ...[31, 32, 799, missingAvatar ? null : 143].map((avatarId, i) => ({ id: `member00000${i}`, username: `member_${i}`, displayName: ['Maya', 'June', 'Theo', missingAvatar ? 'Missing avatar (test)' : 'Avery'][i], avatarId }))].map((m, i) => ({ ...m, owner: i === 0 }));
  const space = { id: 'space1234567', name: 'Avatar test fixture', ownerId: account.id };
  const channel = { id: 'channel12345', spaceId: space.id, name: 'general', private: false };
  const messages = members.map((m, i) => ({ id: `message${i}`, clientMessageId: `client${i}`, channelId: channel.id, seq: String(i + 1), author: { id: m.id, name: m.displayName, isGuest: false, avatarId: m.avatarId }, content: { version: 1, type: 'text', text: ['Explicit test fixture — these are not real accounts or messages.', 'The same saved avatar appears beside my name everywhere.', 'Tile 32 starts the second row of the collection.', 'Tile 799 is the final avatar in the collection.', missingAvatar ? 'Deliberately incomplete response to test the initials fallback.' : 'Existing accounts get a saved default profile picture too.'][i] }, createdAt: '2026-09-30T12:00:00Z' }));
  const history = { space, channel, messages, cursor: '5', hasMore: false };
  const author = () => ({ id: account.id, name: account.displayName, isGuest: false, avatarId: account.avatarId });
  const original = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return original(input, options);
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/account/profile') { Object.assign(account, JSON.parse(options.body)); return Response.json(account); }
    if (path === '/api/spaces') return Response.json({ spaces: [space], limits: { ownedSpaces: 5, totalSpaces: 10, channelsPerSpace: 25 } });
    if (path.endsWith('/members')) return Response.json({ members });
    if (path === '/api/spaces/' + space.id) return Response.json({ space, channels: [channel], members });
    if (path.endsWith('/status')) return Response.json({ enabled: true });
    if (path.endsWith('/session')) return Response.json({ token: 'fixture-only', author: author() });
    if (path.endsWith('/messages')) return Response.json(history);
    return Response.json({ error: 'Unexpected fixture request: ' + path }, { status: 400 });
  };
  window.WebSocket = class extends EventTarget {
    constructor() { super(); queueMicrotask(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() })); }
    frame(frame) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(frame) })); }
    send(raw) {
      const r = JSON.parse(raw);
      if (r.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (r.type === 'subscribe') {
        const event = r.kind === 'chat' ? { type: 'ready', cursor: '5' }
          : r.kind === 'presence' ? { type: 'snapshot', members: r.userIds.map(userId => ({ userId, status: 'online' })) }
          : { type: 'snapshot', participants: members.slice(0, 3).map((m, i) => ({ id: `call-session-${i}`, name: m.displayName, avatarId: m.avatarId, muted: false, deafened: false })), revision: 1 };
        this.frame({ type: 'event', id: r.id, event }); this.frame({ type: 'subscribed', id: r.id });
      }
      if (r.type === 'command') this.frame({ type: 'result', id: r.id, status: 200, body: r.operation === 'status' ? { enabled: true } : { token: 'fixture-only', author: author() } });
    }
    close() {}
  };
}
writeFileSync(init, `(${fixture.toString()})();`);
const session = `avatars-${process.pid}`;
const browser = (...args) => execFileSync('agent-browser', ['--session', session, '--init-script', init, ...args], { encoding: 'utf8', timeout: 40000 });
const evaluate = code => JSON.parse(browser('eval', code));
const wait = condition => browser('wait', '--fn', condition);
try {
  browser('open', `${origin}spaces?space=space1234567&channel=channel12345`);
  browser('set', 'viewport', '1280', '900', '2');
  wait('!document.querySelector(".chat-initial-messages") && document.querySelectorAll(".chat-avatar [data-avatar-id]").length === 5 && document.querySelectorAll(".participant-avatar [data-avatar-id]").length === 3');
  assert.deepEqual(evaluate('[...document.querySelectorAll(".chat-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799, 143]);
  assert.deepEqual(evaluate('[...document.querySelectorAll(".member-presence-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799, 143]);
  assert.deepEqual(evaluate('[...document.querySelectorAll(".voice-stack-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32]);
  assert.equal(evaluate('document.querySelector(".account-avatar [data-avatar-id]").dataset.avatarId'), '0');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-avatar")].every(e => e.querySelector("[data-avatar-id]"))'), true, 'Every account has a saved profile picture');
  assert.equal(evaluate('document.querySelector(".account-avatar .presence-dot").getAttribute("aria-label")'), 'Online');
  assert.equal(evaluate('[...document.querySelectorAll("[data-avatar-id]")].every(e => getComputedStyle(e).backgroundColor === "rgba(0, 0, 0, 0)" && getComputedStyle(e.parentElement).backgroundColor === "rgba(0, 0, 0, 0)")'), true, 'Image avatars and their containers must have transparent backing');
  assert.equal(evaluate('[...document.querySelectorAll("[data-avatar-id]")].every(e => getComputedStyle(e).backgroundImage.endsWith(`/images/avatars/v2/${e.dataset.avatarId}.svg")`))'), true);
  browser('eval', 'Promise.all([0,31,32,799,143].map(id => new Promise((resolve,reject) => { const image = new Image(); image.onload=()=>resolve(true); image.onerror=reject; image.src=`/images/avatars/v2/${id}.svg`; })))');
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-desktop.png'));
  browser('click', '.space-menu summary');
  browser('click', '.space-actions button');
  wait('document.querySelectorAll(".member-avatar [data-avatar-id]").length === 5');
  assert.equal(evaluate('[...document.querySelectorAll(".member-avatar [data-avatar-id]")].every(e => getComputedStyle(e.parentElement).backgroundColor === "rgba(0, 0, 0, 0)")'), true);
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-members-settings.png'));
  browser('click', '[aria-label="Close Manage space"]');
  browser('click', '.account-profile');
  browser('fill', '#display-name', 'Renamed Alex');
  browser('click', 'button[type="submit"]');
  wait('document.querySelector(".account-name")?.textContent === "Renamed Alex"');
  assert.equal(evaluate('document.querySelector(".account-avatar [data-avatar-id]").dataset.avatarId'), '0');
  browser('set', 'viewport', '390', '844', '2');
  browser('eval', 'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
  assert.deepEqual(evaluate('[...document.querySelectorAll(".chat-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799, 143]);
  assert.equal(evaluate('[...document.querySelectorAll("[data-avatar-id]")].every(e => getComputedStyle(e.parentElement).backgroundColor === "rgba(0, 0, 0, 0)")'), true);
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-narrow-members.png'));
  browser('click', '.member-list-toggle');
  wait('!document.querySelector(".space-member-presence")');
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-narrow.png'));

  browser('open', `${origin}spaces?space=space1234567&channel=channel12345&missing-avatar-test=1`);
  wait('!document.querySelector(".chat-initial-messages") && document.querySelectorAll(".chat-avatar").length === 5');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-avatar")].at(-1).textContent'), 'M');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-avatar")].at(-1).querySelector("[data-avatar-id]")'), null);
  assert.notEqual(evaluate('getComputedStyle([...document.querySelectorAll(".chat-avatar")].at(-1).firstElementChild).backgroundColor'), 'rgba(0, 0, 0, 0)', 'Initials retain a readable neutral backing for incomplete responses');
  // Review enlarged real components outside the compact chat layout, then every
  // design. These temporary test surfaces are not product gallery/upload features.
  browser('set', 'viewport', '1280', '900', '2');
  browser('eval', `(() => {
    const stage = document.createElement('section'); stage.id = 'vector-review';
    stage.style.cssText = 'position:fixed;inset:0;z-index:99999;background:#0C0D0F;color:#F3F4F5;padding:32px;overflow:auto';
    stage.innerHTML = '<h1 style="font-size:24px;margin-bottom:24px">Rendering comparison · saved avatar IDs unchanged</h1><div id="comparison" style="display:grid;grid-template-columns:120px repeat(4, 240px);gap:16px;align-items:center"></div>';
    document.body.append(stage);
    const grid = stage.querySelector('#comparison');
    for (const version of ['Previous 64px bitmap', 'New vector paths']) {
      const label = document.createElement('p'); label.textContent = version; grid.append(label);
      for (const id of [0,31,32,799]) {
        const tile = document.querySelector('[data-avatar-id="'+id+'"]').cloneNode(true);
        tile.style.width='240px'; tile.style.height='240px'; tile.style.borderRadius='50%';
        if (version.startsWith('Previous')) {
          tile.style.backgroundImage='url(/images/avatars/capers-v1.webp)';
          tile.style.backgroundSize='3200% 2500%';
          tile.style.backgroundPosition=(id%32)*100/31+'% '+Math.floor(id/32)*100/24+'%';
        }
        grid.append(tile);
      }
    }
    const bitmap = new Image(); bitmap.src='/images/avatars/capers-v1.webp'; return bitmap.decode();
  })()`);
  browser('eval', 'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))');
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-enlarged.png'));
  // All 800 must load, retain alpha outside the circle, and contain visible art.
  assert.equal(evaluate(`(async () => {
    for (let id=0; id<800; id++) {
      const image = new Image(); image.src='/images/avatars/v2/'+id+'.svg'; await image.decode();
      const canvas=document.createElement('canvas'); canvas.width=canvas.height=32;
      const ctx=canvas.getContext('2d'); ctx.drawImage(image,0,0,32,32);
      if (ctx.getImageData(0,0,1,1).data[3] !== 0 || ctx.getImageData(16,16,1,1).data[3] !== 255) throw Error('Invalid alpha for '+id);
    }
    return true;
  })()`), true);
  browser('set', 'viewport', '1280', '1500', '2');
  browser('eval', `(() => {
    const stage=document.querySelector('#vector-review');
    stage.innerHTML='<h1 style="font-size:24px;margin-bottom:24px">100 vector designs · IDs 0–99 · artwork review</h1><div id="designs" style="display:grid;grid-template-columns:repeat(10,1fr);gap:12px"></div>';
    for(let id=0;id<100;id++) {
      const figure=document.createElement('figure'); figure.style.margin='0';
      figure.innerHTML='<img width="100" height="100" src="/images/avatars/v2/'+id+'.svg"><figcaption style="text-align:center">'+id+'</figcaption>';
      stage.querySelector('#designs').append(figure);
    }
    return Promise.all([...stage.querySelectorAll('img')].map(img=>img.decode()));
  })()`);
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-vector-collection.png'));
  console.log('PASS: saved SVG IDs in chat, members, voice roster/stack and account; transparent backing; missing-response fallback; presence; profile rename; desktop, narrow and enlarged Chromium. Mock API, no live voice.');
} catch (error) {
  console.error(browser('snapshot'));
  throw error;
} finally {
  browser('close'); rmSync(scratch, { recursive: true, force: true });
}
