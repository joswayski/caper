// Focused browser regression with disposable API/gateway mocks; never contacts production.
// NAVIGATION_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-desktop-navigation.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
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
const artifacts = process.env.NAVIGATION_TEST_MOBILE_ARTIFACTS;
if (artifacts) mkdirSync(artifacts, { recursive: true });
const screenshot = name => { if (artifacts) browser('screenshot', `${artifacts}/${name}.png`); };
let socket;

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
  // Close the member panel carried across from the desktop layout.
  browser('click', '.member-list-close');
  wait('!document.querySelector(".space-member-presence")');
  assert.equal(evaluate('document.body.textContent.includes("TEST FIXTURE design history")'), true);
  assert.equal(evaluate('document.querySelector(".chat-heading > .voice-actions .member-list-toggle")'), null, 'Mobile members belongs in the channel menu');
  assert.equal(evaluate('document.querySelector(".chat-heading > .chat-pins-toggle")'), null, 'Mobile Pins must not have a dedicated header button');
  assert.equal(evaluate('document.querySelectorAll(".chat-channel-menu .chat-pins-toggle").length'), 1, 'Pins remains available inside the channel menu');
  assert.equal(evaluate('document.querySelector(".navigation-toggle").getAttribute("aria-label")'), 'Back to Browse');
  assert.equal(evaluate('document.querySelector(".navigation-toggle").getBoundingClientRect().width'), 44);

  // Send real Chromium touch input through the agent-browser session's CDP.
  socket = new WebSocket(browser('get', 'cdp-url').cdpUrl);
  await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener('message', event => {
    const response = JSON.parse(event.data), request = pending.get(response.id);
    if (!request) return;
    pending.delete(response.id);
    if (response.error) request.reject(new Error(response.error.message));
    else request.resolve(response.result);
  });
  const cdp = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const { targetInfos } = await cdp('Target.getTargets');
  const target = targetInfos.find(target => target.type === 'page' && target.url.startsWith(base));
  assert.ok(target);
  const { sessionId } = await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true });
  await cdp('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 2 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  const touch = (type, x, y) => cdp('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' || type === 'touchCancel' ? [] : [{ x, y }] }, sessionId);
  const swipe = async (x, y, dx, dy = 0, end = 'touchEnd') => {
    await touch('touchStart', x, y);
    for (let step = 1; step <= 6; step++) await touch('touchMove', x + dx * step / 6, y + dy * step / 6);
    await touch(end);
    await new Promise(resolve => setTimeout(resolve, 80));
  };
  const browsing = () => evaluate('document.querySelector(".spaces-room").classList.contains("navigation-open")');
  const point = selector => evaluate(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  browser('fill', '#chat-message', 'mobile draft survives both swipe directions');
  browser('click', '.chat-heading'); // Dismiss the keyboard/focus without clearing the draft.
  const currentUrl = evaluate('location.href');
  const reads = evaluate('navigationFixture.historyReads.design123456');
  const p = point('.chat-messages');
  for (const [dx, dy, end] of [[60, 0], [-100, 0], [90, 140], [100, 0, 'touchCancel']]) {
    await swipe(p.x - 60, p.y, dx, dy, end);
    assert.equal(browsing(), false, 'Short, wrong-direction, vertical and canceled gestures must not navigate');
  }
  const fingers = [{ id: 0, x: p.x - 60, y: p.y }, { id: 1, x: p.x - 60, y: p.y + 40 }];
  await cdp('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: fingers }, sessionId);
  await cdp('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: fingers.map(finger => ({ ...finger, x: finger.x + 100 })) }, sessionId);
  await touch('touchEnd');
  assert.equal(browsing(), false, 'Multi-touch must not trigger one-finger navigation');
  const composer = point('#chat-message');
  await swipe(composer.x - 50, composer.y, 100);
  assert.equal(browsing(), false, 'Editing/text selection in the composer must not navigate');
  evaluate('window.getSelection()?.removeAllRanges(); document.activeElement.blur()');
  screenshot('mobile-chat');
  await swipe(p.x - 60, p.y, 68);
  assert.equal(browsing(), true, 'Right swipe above threshold must reveal Browse');
  screenshot('mobile-browse');
  const row = point('.channel-select:not([aria-current="page"])');
  await swipe(row.x + 60, row.y, -110);
  assert.equal(browsing(), false, 'Left swipe on a channel row must return to the current chat, not select that row');
  assert.equal(evaluate('location.href'), currentUrl);
  assert.equal(evaluate('document.querySelector("#chat-message").value'), 'mobile draft survives both swipe directions');
  assert.equal(evaluate('navigationFixture.historyReads.design123456'), reads, 'Showing and hiding Browse must not refetch the current history');

  browser('focus', '.chat-channel-menu summary');
  browser('press', 'Enter');
  wait('document.querySelector(".chat-channel-menu").open');
  screenshot('mobile-channel-menu');
  await swipe(p.x - 60, p.y, 100);
  assert.equal(browsing(), false, 'An open dropdown must not pass a gesture through to navigation');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu").open'), false, 'Outside touch dismisses the dropdown');
  browser('click', '.chat-channel-menu summary');
  browser('press', 'Escape');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu").open'), false);
  assert.equal(evaluate('document.activeElement === document.querySelector(".chat-channel-menu summary")'), true);
  browser('click', '.chat-channel-menu summary');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu .chat-pins-toggle").getAttribute("aria-label")'), 'Pins, 0');
  browser('click', '.chat-channel-menu .chat-pins-toggle');
  wait('document.querySelector(".chat-pins")?.textContent.includes("No pinned messages.")');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu").open'), false, 'Choosing Pins closes the menu');
  screenshot('mobile-empty-pins');
  browser('click', '.chat-channel-menu summary');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu .chat-pins-toggle").textContent'), 'Messages');
  browser('click', '.chat-channel-menu .chat-pins-toggle');
  wait('!document.querySelector(".chat-pins") && !!document.querySelector(".chat-message")');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu").open'), false, 'Choosing Messages closes the menu');
  assert.equal(evaluate('document.querySelector("#chat-message").value'), 'mobile draft survives both swipe directions', 'Pins must preserve the conversation draft');
  browser('click', '.chat-channel-menu summary');
  browser('click', '.chat-channel-menu .member-list-toggle');
  wait('!!document.querySelector(".space-member-presence")');
  assert.equal(evaluate('document.querySelector(".chat-channel-menu").open'), false, 'Choosing Members closes the menu');
  screenshot('mobile-members');
  await swipe(p.x - 60, p.y, 100);
  assert.equal(browsing(), false, 'Member overlay must not pass swipes through');
  if (evaluate('!!document.querySelector(".space-member-presence")')) browser('click', '.member-list-close');
  browser('click', '.navigation-toggle');
  assert.equal(browsing(), true, 'Back button must open Browse');
  browser('click', '.channel-select[aria-current="page"]');
  assert.equal(browsing(), false);
  assert.equal(evaluate('document.querySelector("#chat-message").value'), 'mobile draft survives both swipe directions');
  browser('set', 'viewport', '320', '568', '2');
  evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
  browser('click', '.chat-channel-menu summary');
  assert.equal(evaluate('document.documentElement.scrollWidth <= innerWidth'), true, 'Channel dropdown must fit a small phone');
  screenshot('mobile-small-menu');
  browser('press', 'Escape');
  browser('set', 'viewport', '390', '844', '2');
  if (process.env.NAVIGATION_TEST_NARROW_SCREENSHOT) browser('screenshot', process.env.NAVIGATION_TEST_NARROW_SCREENSHOT);
  console.log('PASS: desktop navigation/presence; Chromium touch: bidirectional Browse swipes, threshold/direction/vertical/cancel guards, composer and overlays, row release-click protection, draft/history retention, Back button, Pins/Messages/Members dropdown, empty pins and 320px layout.');
} finally {
  socket?.close();
  try { browser('close'); } catch {}
  rmSync(directory, { recursive: true, force: true });
}
