// Browser regression using explicitly mocked API responses. Never writes real data.
// Run with the dev server: SPACES_TEST_WEB_URL=http://localhost:3000/spaces node scripts/test-space-controls.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const url = process.env.SPACES_TEST_WEB_URL ?? 'http://localhost:3000/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname), 'Use a loopback preview');
const artifacts = process.env.SPACES_TEST_ARTIFACTS && resolve(process.env.SPACES_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(join(tmpdir(), 'caper-space-controls-'));
const init = join(directory, 'fixture.js');
function fixture() {
  if (location.protocol === 'about:') return;
  const saved = new URL(location.href).searchParams.get('width') ?? '240';
  localStorage.setItem('caper:channel-sidebar-width', saved);
  const account = { id: 'owner1234567', username: 'fixture_owner', displayName: 'Fixture owner', debugEnabled: new URL(location.href).searchParams.has('debug') };
  const space = { id: 'space1234567', name: 'Disposable UI fixture', ownerId: account.id };
  const channel = { id: 'channel12345', spaceId: space.id, name: 'fixture-channel', private: true };
  const members = [{ ...account, owner: true }, ...Array.from({ length: 29 }, (_, index) => ({
    id: `member${String(index).padStart(6, '0')}`, username: `member_${index}`, displayName: `Fixture member ${index + 1}`, owner: false,
  }))];
  const control = window.spaceControlFixture = { deletes: [], updates: [], fail: false, release: null, frames: [], subscriptions: {} };
  let deletedChannel = false, deletedSpace = false;
  const originalFetch = window.fetch.bind(window);
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    subscriptions = new Map();
    constructor(socketUrl, protocols) {
      super();
      if (!String(socketUrl).includes('/api/chat/events')) return new NativeSocket(socketUrl, protocols);
      control.setPresence = (status) => {
        for (const request of this.subscriptions.values()) {
          if (request.kind === 'presence') this.frame({ type: 'event', id: request.id, event: { type: 'snapshot', members: request.userIds.map(userId => ({ userId, status })) } });
        }
      };
      queueMicrotask(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (request.type === 'unsubscribe') { this.subscriptions.delete(request.id); delete control.subscriptions[request.id]; }
      if (request.type === 'subscribe') {
        this.subscriptions.set(request.id, request);
        control.subscriptions[request.id] = request;
        const event = request.kind === 'chat'
          ? { type: 'ready', cursor: request.after ?? '0' }
          : request.kind === 'presence'
            ? { type: 'snapshot', members: request.userIds.map((userId, index) => ({ userId, status: ['online', 'idle', 'offline'][index % 3] })) }
            : { type: 'snapshot', participants: [], revision: 1 };
        if (request.kind !== 'presence' || !control.holdPresence) this.frame({ type: 'event', id: request.id, event });
        this.frame({ type: 'subscribed', id: request.id });
      }
    }
    frame(value) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(value) })); }
    close() {}
  };
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
    if (options.method === 'PATCH') {
      const body = JSON.parse(options.body);
      control.updates.push({ path, body });
      const target = path.includes('/channels/') ? channel : space;
      Object.assign(target, body);
      return Response.json(target);
    }
    if (options.method === 'DELETE') {
      control.deletes.push(path);
      await new Promise(resolve => { control.release = resolve; });
      if (control.fail) return Response.json({ error: 'Test-only deletion failure' }, { status: 503 });
      if (path.includes('/channels/')) deletedChannel = true;
      else deletedSpace = true;
      return new Response(null, { status: 204 });
    }
    if (path === '/api/account/me') {
      await new Promise(resolve => setTimeout(resolve, 350));
      return Response.json(account);
    }
    if (path === '/api/spaces') return Response.json({ spaces: deletedSpace ? [] : [space], limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 } });
    if (path.endsWith('/members')) return Response.json({ members: [{ ...account, owner: true }] });
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels: deletedChannel ? [] : [channel], members });
    if (path.endsWith('/messages')) return Response.json({ space, channel, messages: [], cursor: '0', hasMore: false });
    return Response.json({ error: 'Disabled in UI fixture' }, { status: 503 });
  };
  function sample() {
    const room = document.querySelector('.call-room');
    if (room) {
      const bounds = selector => {
        const r = document.querySelector(selector).getBoundingClientRect();
        return [r.x, r.y, r.width, r.height];
      };
      const frame = { loading: !!document.querySelector('.spaces-loading'), geometry: ['.call-header', '.call-room', '.space-rail', '.people-panel'].map(bounds) };
      if (JSON.stringify(frame) !== JSON.stringify(control.frames.at(-1))) control.frames.push(frame);
    }
    if (!control.frames.some(frame => !frame.loading)) requestAnimationFrame(sample);
  }
  requestAnimationFrame(sample);
}
writeFileSync(init, `(${fixture.toString()})()`);
const args = ['--session', 'space-controls-test', '--init-script', init];
function browser(...command) {
  const result = JSON.parse(execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);
const screenshot = name => { if (artifacts) browser('screenshot', '--full', `${artifacts}/${name}.png`); };
const modal = '.delete-confirmation';
const opens = () => evaluate('document.querySelectorAll(".space-dialog[open]").length');
function openOverview() {
  browser('focus', '[aria-label="Manage fixture-channel"]');
  browser('press', 'Enter');
  browser('click', '.danger-outline');
  wait('!!document.querySelector(".delete-confirmation")');
}
try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  for (const [viewport, saved, expected] of [[1280, '240', 240], [1280, '440', 440], [800, '440', 362], [1280, 'invalid', 280]]) {
    browser('set', 'viewport', String(viewport), '900', '2');
    browser('open', `${url}?space=space1234567&channel=channel12345&width=${saved}`);
    wait('!!document.querySelector(".channel-navigation")');
    wait('spaceControlFixture.frames.some(frame => !frame.loading)');
    const frames = evaluate('spaceControlFixture.frames');
    assert.ok(frames.some(f => f.loading) && frames.some(f => !f.loading));
    for (const f of frames) {
      assert.equal(f.geometry[3][2], expected, `Wrong sidebar width during ${f.loading ? 'loading' : 'loaded'} state`);
      assert.deepEqual(f.geometry, frames.at(-1).geometry, 'Loading shell moved');
    }
  }
  wait('document.querySelectorAll(".space-member-presence li").length === 25 && !document.querySelector(".member-presence-connecting")');
  assert.ok(evaluate('(() => { const a = document.querySelector(".channel-navigation > header").getBoundingClientRect(), b = document.querySelector(".chat-heading").getBoundingClientRect(); return a.top === b.top && a.bottom === b.bottom; })()'), 'Space and channel headers must align');
  assert.ok(evaluate('document.querySelector(".space-member-presence").getBoundingClientRect().left >= document.querySelector(".stage").getBoundingClientRect().right'), 'Desktop members must be on the right');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).find(s => s.kind === "presence" && s.userIds.length > 1).userIds.length'), 25);
  assert.equal(evaluate('document.querySelector(".account-avatar .presence-dot").getAttribute("aria-label")'), 'Online');
  assert.equal(evaluate('document.querySelectorAll(".space-member-presence small").length'), 0, 'Statuses belong on the dots, not text rows');
  assert.equal(evaluate('getComputedStyle(document.querySelector(".member-presence-heading")).borderBottomWidth'), '0px');
  assert.equal(evaluate('document.querySelector(".channel-section-toggle .section-count").textContent'), '1');
  assert.equal(evaluate('document.querySelector(".member-presence-heading .section-count").textContent'), '30', 'Member count must include every page');
  screenshot('gateway-merged-members-desktop');
  for (const status of ['idle', 'offline', 'online']) {
    evaluate(`spaceControlFixture.setPresence('${status}')`);
    wait(`document.querySelector('.account-avatar .presence-dot').dataset.status === '${status}'`);
    assert.equal(evaluate('document.querySelector(".space-member-presence .presence-dot").dataset.status'), status);
    screenshot(`presence-${status}`);
  }
  browser('click', '.member-presence-pages button:last-child');
  wait('document.querySelectorAll(".space-member-presence li").length === 5 && !document.querySelector(".member-presence-connecting")');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).filter(s => s.kind === "presence").length'), 2);
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).find(s => s.kind === "presence" && s.userIds.length > 1).userIds.length'), 5);
  const expandedChatWidth = evaluate('document.querySelector(".stage").getBoundingClientRect().width');
  browser('click', '.member-list-toggle');
  wait('!Object.values(spaceControlFixture.subscriptions).some(s => s.kind === "presence" && s.userIds.length > 1)');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).filter(s => s.kind === "presence").length'), 1, 'Own presence must remain subscribed with members hidden');
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null, 'Hidden members must not retain a sidebar');
  assert.equal(evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded")'), 'false');
  assert.ok(evaluate('document.querySelector(".stage").getBoundingClientRect().width') > expandedChatWidth, 'Chat must reclaim the member column');
  browser('click', '.channel-section-toggle');
  assert.ok(evaluate('document.querySelector("#space-channel-list").hidden'));
  assert.equal(evaluate('document.querySelector(".channel-section-toggle .section-count").textContent'), '1');
  screenshot('members-collapsed');
  browser('click', '.channel-section-toggle');
  browser('click', '.member-list-toggle');
  wait('document.querySelectorAll(".space-member-presence li").length === 25');
  assert.equal(evaluate('document.querySelector(".member-presence-heading .section-count").textContent'), '30');
  assert.ok(evaluate('[...document.querySelectorAll(".space-member-presence .presence-dot")].every(node => ["online", "idle", "offline"].includes(node.dataset.status))'), 'Reopening must restore live subscriptions');
  assert.equal(evaluate('document.querySelector(".space-member-presence").textContent.includes("Updating")'), false);
  browser('set', 'viewport', '390', '844', '2');
  screenshot('members-narrow-open');
  browser('click', '.member-list-toggle');
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null);
  screenshot('members-narrow-hidden');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Narrow member navigation must not overflow');
  screenshot('gateway-merged-members-narrow');
  browser('set', 'viewport', '1280', '900', '2');
  for (const label of ['Create space', 'Create channel']) {
    browser('click', `[aria-label="${label}"]`);
    wait('!!document.querySelector(".space-dialog[open]")');
    browser('click', '.space-field input');
    assert.equal(opens(), 1, 'Clicking inside must not dismiss');
    screenshot(label === 'Create space' ? 'create-space-dialog' : 'create-channel-dialog');
    browser('mouse', 'move', '10', '10');
    browser('mouse', 'down', 'left');
    browser('mouse', 'up', 'left');
    assert.equal(opens(), 0, `${label} must dismiss on backdrop click`);
  }
  openOverview();
  assert.equal(opens(), 2);
  assert.equal(evaluate('document.activeElement.textContent'), 'Cancel');
  assert.equal(evaluate('getComputedStyle(document.activeElement).outlineStyle'), 'solid');
  screenshot('gateway-merged-delete-confirmation');
  browser('press', 'Enter');
  assert.equal(opens(), 1, 'Immediate Enter must cancel, not delete');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 0);
  browser('click', '.danger-outline');
  browser('press', 'Escape');
  assert.equal(opens(), 1, 'Escape must close only the confirmation');
  assert.equal(evaluate('document.activeElement.className'), 'danger-outline');
  browser('fill', '.space-field input', '   Fresh Plans   ');
  browser('press', 'Enter');
  wait('!document.querySelector(".channel-save-bar")');
  assert.equal(evaluate('document.querySelector(".space-field input").value'), 'fresh-plans');
  assert.deepEqual(evaluate('spaceControlFixture.updates.at(-1).body'), { name: 'fresh-plans', private: true });
  browser('click', '.danger-outline');
  browser('mouse', 'move', '10', '10');
  browser('mouse', 'down', 'left');
  browser('mouse', 'up', 'left');
  assert.equal(opens(), 1, 'Backdrop dismissal must preserve settings');
  browser('dblclick', '.danger-outline');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 0, 'Double-click opener must not delete');
  assert.equal(opens(), 2);
  evaluate('document.querySelector(".delete-confirmation .danger").dispatchEvent(new MouseEvent("click", {bubbles:true, detail:2}))');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 0, 'Second click must not confirm');
  browser('click', `${modal} .secondary`);
  browser('click', '.danger-outline');
  evaluate('spaceControlFixture.fail = true');
  browser('click', `${modal} .danger`);
  wait('spaceControlFixture.release !== null');
  browser('press', 'Escape');
  assert.equal(opens(), 2, 'Pending deletion must retain its result surface');
  assert.equal(evaluate('document.querySelector(".delete-confirmation .danger").disabled'), true);
  evaluate('spaceControlFixture.release()');
  wait('!!document.querySelector(".delete-confirmation [role=alert]")');
  assert.equal(evaluate('document.querySelector(".delete-confirmation [role=alert]").textContent'), 'Test-only deletion failure');
  evaluate('spaceControlFixture.fail = false; spaceControlFixture.release = null');
  browser('click', `${modal} .danger`);
  wait('spaceControlFixture.release !== null');
  evaluate('spaceControlFixture.release()');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.deepEqual(evaluate('spaceControlFixture.deletes'), Array(2).fill('/api/spaces/space1234567/channels/channel12345'));
  browser('click', '.space-menu summary');
  browser('click', '.space-actions button');
  browser('fill', '.space-field input', '   Renamed studio   ');
  browser('press', 'Enter');
  wait('document.querySelector(".space-field input").value === "Renamed studio"');
  assert.deepEqual(evaluate('spaceControlFixture.updates.at(-1).body'), { name: 'Renamed studio' });
  browser('fill', '.space-field input', '   Renamed studio   ');
  browser('press', 'Tab');
  assert.equal(evaluate('document.querySelector(".space-field input").value'), 'Renamed studio', 'Whitespace-only edits normalize on blur even without saving');
  browser('click', '.danger-outline');
  assert.match(evaluate('document.querySelector(".delete-confirmation").textContent'), /All its channels and their messages will disappear from the space\. This cannot be undone/);
  browser('press', 'Enter');
  assert.equal(opens(), 1, 'Immediate Enter also cancels space deletion');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 2);
  browser('click', '.danger-outline');
  browser('click', `${modal} .danger`);
  wait('spaceControlFixture.deletes.length === 3');
  evaluate('spaceControlFixture.release()');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.equal(evaluate('spaceControlFixture.deletes.at(-1)'), '/api/spaces/space1234567');
  for (const enabled of [false, true]) {
    browser('set', 'viewport', '1280', '900', '2');
    browser('open', `${url}?space=space1234567&channel=channel12345${enabled ? '&debug' : ''}`);
    wait('!!document.querySelector(".account-avatar")');
    browser('click', '[aria-label="User Settings"]');
    assert.equal(evaluate('[...document.querySelectorAll("button")].some(b => b.textContent === "Audio diagnostics")'), enabled);
    if (enabled) {
      browser('find', 'role', 'button', 'click', '--name', 'Audio diagnostics', '--exact');
      wait('!!document.querySelector(".audio-debug")');
      assert.match(evaluate('document.querySelector(".audio-debug").textContent'), /No microphone capture started/);
      assert.equal(evaluate('JSON.parse(document.querySelector(".audio-debug pre").textContent).captureAttempt'), 'not-started');
    }
  }
  console.log('PASS: debug-enabled account sees diagnostics; other accounts do not; unused capture is explicit.');
  console.log('PASS: stable loading geometry, scoped member pagination/unsubscribe and narrow layout, safe confirmation focus/Enter/dismissal/double-click, trimmed name updates, pending/failure/retry, channel and space deletion (mock API/gateway).');
} finally {
  try { browser('close'); } finally { rmSync(directory, { recursive: true, force: true }); }
}
