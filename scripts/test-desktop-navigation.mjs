// Focused browser regression with disposable API/gateway mocks; never contacts production.
// NAVIGATION_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-desktop-navigation.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const base = process.env.NAVIGATION_TEST_WEB_URL ?? 'http://localhost:5174/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(base).hostname), 'Use a loopback dev server');
const url = new URL(base);
url.searchParams.set('space', 'space1234567');
url.searchParams.set('channel', 'general12345');
const directory = mkdtempSync(join(tmpdir(), 'caper-navigation-'));
const init = join(directory, 'fixture.js');

function fixture() {
  if (location.protocol === 'about:') return;
  const account = { id: 'member123456', username: 'member', displayName: 'TEST FIXTURE member' };
  const space = { id: 'space1234567', name: 'TEST FIXTURE navigation', ownerId: 'owner1234567' };
  const channels = [
    { id: 'general12345', spaceId: space.id, name: 'general', private: false, joined: true },
    { id: 'design123456', spaceId: space.id, name: 'design', private: false, joined: true },
  ];
  const members = [{ ...account, owner: false }];
  const control = window.navigationFixture = { historyReads: {}, presenceSubscribes: 0, releases: [], holdDesign: true };
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    constructor(socketUrl, protocols) {
      super();
      if (!String(socketUrl).includes('/api/chat/events')) return new NativeSocket(socketUrl, protocols);
      queueMicrotask(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (request.type !== 'subscribe') return;
      if (request.kind === 'presence') control.presenceSubscribes++;
      const event = request.kind === 'chat'
        ? { type: 'ready', cursor: request.after ?? '1' }
        : request.kind === 'presence'
          ? { type: 'snapshot', members: request.userIds.map(userId => ({ userId, status: 'online' })) }
          : { type: 'snapshot', participants: [], revision: 1 };
      this.frame({ type: 'event', id: request.id, event });
      this.frame({ type: 'subscribed', id: request.id });
    }
    frame(value) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(value) })); }
    close() {}
  };
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/spaces') return Response.json({ spaces: [space], invitations: [], limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 } });
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels, members });
    if (path === '/api/dms') return Response.json({ conversations: [] });
    if (path === '/api/chat/session') return Response.json({ token: 'fixture', author: { id: account.id, name: account.displayName, isGuest: false } });
    if (path.endsWith('/media/status')) return Response.json({ enabled: false });
    const match = path.match(/\/api\/chat\/channels\/([^/]+)\/messages$/);
    if (match) {
      const channel = channels.find(item => item.id === match[1]);
      control.historyReads[channel.id] = (control.historyReads[channel.id] ?? 0) + 1;
      if (channel.id === 'design123456' && control.holdDesign) await new Promise(resolve => control.releases.push(resolve));
      return Response.json({ space, channel, cursor: '1', hasMore: false, messages: [{
        id: `message-${channel.id}`, channelId: channel.id, seq: '1',
        clientMessageId: `00000000-0000-4000-8000-${channel.id === 'general12345' ? '000000000001' : '000000000002'}`,
        createdAt: '2026-10-04T12:00:00Z', author: { id: account.id, name: account.displayName, isGuest: false },
        content: { version: 1, type: 'text', text: `TEST FIXTURE ${channel.name} history` },
      }] });
    }
    return Response.json({ error: 'TEST FIXTURE: disabled endpoint' }, { status: 503 });
  };
}

writeFileSync(init, `(${fixture.toString()})()`);
const session = `nav-${process.pid}`;
const args = ['--session', session, '--init-script', init];
const browser = (...command) => {
  const output = JSON.parse(execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 60_000 }));
  assert.ok(output.success, output.error);
  return output.data;
};
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);

try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  browser('open', url.toString());
  wait('document.querySelector("#chat-message") && !document.querySelector("#chat-message").disabled');
  wait('document.querySelector(".space-member-presence .presence-dot")?.dataset.status === "online"');
  browser('fill', '#chat-message', 'draft survives repeat click');
  evaluate('(window.navigationFixture.messageNode = document.querySelector(".chat-message"), true)');
  const initialPresence = evaluate('navigationFixture.presenceSubscribes');
  const initialGeneralReads = evaluate('navigationFixture.historyReads.general12345');
  browser('click', '[aria-current="page"].channel-select');
  assert.equal(evaluate('document.querySelector("#chat-message").value'), 'draft survives repeat click');
  assert.equal(evaluate('document.querySelector(".chat-message") === navigationFixture.messageNode'), true, 'repeat click must retain rendered history nodes');
  assert.equal(evaluate('navigationFixture.historyReads.general12345'), initialGeneralReads, 'repeat click must not refetch history');

  browser('hover', '.channel-select:not([aria-current="page"])');
  wait('navigationFixture.historyReads.design123456 === 1');
  browser('click', '.channel-select:not([aria-current="page"])');
  assert.equal(evaluate('navigationFixture.historyReads.design123456'), 1, 'hover and click must share the in-flight history request');
  browser('click', '[aria-current="page"].channel-select');
  wait('!document.querySelector("[aria-busy=true].channel-select")');
  evaluate('navigationFixture.releases.splice(0).forEach(release => release())');
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.equal(evaluate('document.querySelector("#space-channel-list [aria-current=page] span").textContent'), 'general', 'stale completion must not replace the restored channel');
  assert.equal(evaluate('document.querySelector("#chat-message").value'), 'draft survives repeat click', 'canceling pending navigation must retain the visible draft');

  evaluate('navigationFixture.holdDesign = false');
  browser('click', '.channel-select:not([aria-current="page"])');
  wait('document.querySelector("#space-channel-list [aria-current=page] span").textContent === "design" && document.body.textContent.includes("TEST FIXTURE design history")');
  assert.equal(evaluate('navigationFixture.presenceSubscribes'), initialPresence, 'same-space channel changes must not restart presence subscriptions');
  assert.equal(evaluate('navigationFixture.historyReads.design123456'), 2, 'a canceled request is not treated as a committed visited history');
  if (process.env.NAVIGATION_TEST_SCREENSHOT) browser('screenshot', process.env.NAVIGATION_TEST_SCREENSHOT);

  browser('set', 'viewport', '390', '844', '2');
  evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
  assert.equal(evaluate('document.documentElement.scrollWidth <= innerWidth'), true);
  // The existing narrow member panel intentionally overlays the conversation.
  // Close it before inspecting the channel history and composer at this width.
  browser('click', '.space-member-presence [aria-label="Close member list"]');
  wait('!document.querySelector(".space-member-presence")');
  assert.equal(evaluate('document.body.textContent.includes("TEST FIXTURE design history")'), true);
  if (process.env.NAVIGATION_TEST_NARROW_SCREENSHOT) browser('screenshot', process.env.NAVIGATION_TEST_NARROW_SCREENSHOT);
  console.log('PASS: repeat-click draft/history retention, shared hover+click request, stale completion cancellation, and stable same-space presence; desktop + narrow Chromium.');
} finally {
  try { browser('close'); } catch {}
  rmSync(directory, { recursive: true, force: true });
}
