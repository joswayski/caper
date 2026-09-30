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
  const account = { id: 'owner1234567', username: 'fixture_owner', displayName: 'Alex', avatarId: 0 };
  const members = [account, ...[31, 32, 799, null].map((avatarId, i) => ({ id: `member00000${i}`, username: `member_${i}`, displayName: ['Maya', 'June', 'Theo', 'Legacy'][i], avatarId }))].map((m, i) => ({ ...m, owner: i === 0 }));
  const space = { id: 'space1234567', name: 'Avatar test fixture', ownerId: account.id };
  const channel = { id: 'channel12345', spaceId: space.id, name: 'general', private: false };
  const messages = members.map((m, i) => ({ id: `message${i}`, clientMessageId: `client${i}`, channelId: channel.id, seq: String(i + 1), author: { id: m.id, name: m.displayName, isGuest: false, avatarId: m.avatarId }, content: { version: 1, type: 'text', text: ['Explicit test fixture — these are not real accounts or messages.', 'The same saved avatar appears beside my name everywhere.', 'Tile 32 starts the second row of the collection.', 'Tile 799 is the final avatar in the collection.', 'Older clients and missing avatars fall back to initials.'][i] }, createdAt: '2026-09-30T12:00:00Z' }));
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
  wait('!document.querySelector(".chat-initial-messages") && document.querySelectorAll(".chat-avatar [data-avatar-id]").length === 4 && document.querySelectorAll(".participant-avatar [data-avatar-id]").length === 3');
  assert.deepEqual(evaluate('[...document.querySelectorAll(".chat-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799]);
  assert.deepEqual(evaluate('[...document.querySelectorAll(".member-presence-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799]);
  assert.deepEqual(evaluate('[...document.querySelectorAll(".voice-stack-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32]);
  assert.equal(evaluate('document.querySelector(".account-avatar [data-avatar-id]").dataset.avatarId'), '0');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-avatar")].at(-1).textContent'), 'L');
  assert.equal(evaluate('document.querySelector(".account-avatar .presence-dot").getAttribute("aria-label")'), 'Online');
  assert.equal(evaluate('[...document.querySelectorAll("[data-avatar-id]")].every(e => getComputedStyle(e).backgroundColor === "rgba(0, 0, 0, 0)" && getComputedStyle(e.parentElement).backgroundColor === "rgba(0, 0, 0, 0)")'), true, 'Image avatars and their containers must have transparent backing');
  assert.notEqual(evaluate('getComputedStyle([...document.querySelectorAll(".chat-avatar")].at(-1).firstElementChild).backgroundColor'), 'rgba(0, 0, 0, 0)', 'Initials retain a readable neutral backing');
  browser('eval', 'new Promise((resolve,reject) => { const image = new Image(); image.onload=()=>image.width===2048&&image.height===1600?resolve(true):reject(Error("Wrong atlas size")); image.onerror=reject; image.src="/images/avatars/capers-v1.webp"; })');
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-desktop.png'));
  browser('click', '.space-menu summary');
  browser('click', '.space-actions button');
  wait('document.querySelectorAll(".member-avatar [data-avatar-id]").length === 4');
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
  assert.deepEqual(evaluate('[...document.querySelectorAll(".chat-avatar [data-avatar-id]")].map(e => Number(e.dataset.avatarId))'), [0, 31, 32, 799]);
  assert.equal(evaluate('[...document.querySelectorAll("[data-avatar-id]")].every(e => getComputedStyle(e.parentElement).backgroundColor === "rgba(0, 0, 0, 0)")'), true);
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-narrow-members.png'));
  browser('click', '.member-list-toggle');
  wait('!document.querySelector(".space-member-presence")');
  if (artifacts) browser('screenshot', join(artifacts, 'avatars-narrow.png'));
  console.log('PASS: saved IDs in chat, members, voice roster/stack and account; zero/row/end tiles; transparent image backing; legacy initials; presence; profile rename; desktop and narrow Chromium. Mock API, no live voice.');
} catch (error) {
  console.error(browser('snapshot'));
  throw error;
} finally {
  browser('close'); rmSync(scratch, { recursive: true, force: true });
}
