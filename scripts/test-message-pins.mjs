// Disposable HTTP/WebSocket fixture + Chromium, not native/device acceptance.
// Start native-parity-fixture.mjs and Vite before running this script.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const web = process.env.MESSAGE_TEST_WEB_URL ?? 'http://127.0.0.1:5174';
const api = process.env.MESSAGE_TEST_API_URL ?? 'http://127.0.0.1:3001';
for (const url of [web, api]) assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (session, ...args) => {
  const chrome = process.env.MESSAGE_TEST_CHROME ? ['--executable-path', process.env.MESSAGE_TEST_CHROME] : [];
  const result = JSON.parse(execFileSync('agent-browser', ['--session', session, ...chrome, ...args, '--json'], { encoding: 'utf8', timeout: 60_000 }));
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (session, source) => browser(session, 'eval', source).result;
const wait = (session, source) => browser(session, 'wait', '--fn', source);
const action = (session, name) => browser(session, 'find', 'role', 'button', 'click', '--name', name, '--exact');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const screenshot = (session, name) => {
  if (!artifacts) return;
  evaluate(session, 'new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
  assert.equal(evaluate(session, 'devicePixelRatio'), 2);
  browser(session, 'screenshot', `${artifacts}/${name}.png`);
};
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, { method: 'POST', headers: { 'content-type': 'application/json', connection: 'close' }, body: JSON.stringify(body) });
  assert.equal(response.status, 200);
}
async function history() {
  return (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers: { authorization: 'Bearer fixture-owner-token', connection: 'close' } })).json();
}

let socket;
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  await control({ reset: true });
  const initial = await history();
  const message = initial.messages.at(-1);
  const row = `[data-message-key="${message.clientMessageId}"]`;
  for (const session of ['pins-check', 'pins-peer']) {
    browser(session, 'open', 'about:blank');
    browser(session, 'set', 'viewport', '1280', '900', '2');
    browser(session, 'cookies', 'set', 'caper_fixture', 'owner', '--url', web, '--path', '/', '--sameSite', 'Lax');
    browser(session, 'open', `${web}/spaces`);
    wait(session, `!!document.querySelector('${row} .chat-message-actions-trigger') && !document.querySelector('.chat-initial-messages')`);
    evaluate(session, 'document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))');
  }
  // Desktop hover/focus behavior needs Chromium launched with a fine pointer.
  if (process.env.MESSAGE_TEST_CHROME) {
    assert.equal(evaluate('pins-check', 'matchMedia("(hover: hover) and (pointer: fine)").matches'), true);
    browser('pins-check', 'hover', '#chat-heading');
    wait('pins-check', `['.chat-add-reaction', '.chat-message-actions-trigger'].every(selector => getComputedStyle(document.querySelector('${row} ' + selector)).opacity === '0')`);
    assert.deepEqual(evaluate('pins-check', `['.chat-add-reaction', '.chat-message-actions-trigger'].map(selector => getComputedStyle(document.querySelector('${row} ' + selector)).opacity)`), ['0', '0']);
    browser('pins-check', 'hover', `${row} p`);
    wait('pins-check', `['.chat-add-reaction', '.chat-message-actions-trigger'].every(selector => getComputedStyle(document.querySelector('${row} ' + selector)).opacity === '1')`);
    assert.deepEqual(evaluate('pins-check', `(() => {
      const buttons = ['.chat-add-reaction', '.chat-message-actions-trigger'].map(selector => document.querySelector('${row} ' + selector));
      const rects = buttons.map(button => button.getBoundingClientRect());
      return [buttons.map(button => getComputedStyle(button).opacity), rects.map(rect => [rect.width, rect.height]), rects[1].left - rects[0].right, buttons.map(button => button.querySelector('svg').getBoundingClientRect().width)];
    })()`), [['1', '1'], [[24, 24], [24, 24]], 2, [14, 14]]);
    browser('pins-check', 'hover', '#chat-heading');
    browser('pins-check', 'focus', `${row} .chat-add-reaction`);
    wait('pins-check', `['.chat-add-reaction', '.chat-message-actions-trigger'].every(selector => getComputedStyle(document.querySelector('${row} ' + selector)).opacity === '1')`);
    assert.deepEqual(evaluate('pins-check', `['.chat-add-reaction', '.chat-message-actions-trigger'].map(selector => getComputedStyle(document.querySelector('${row} ' + selector)).opacity)`), ['1', '1'], 'Keyboard focus also reveals actions');
    evaluate('pins-check', 'document.activeElement.blur()');
  }
  const directGap = () => evaluate('pins-check', `document.querySelector('.direct-section').getBoundingClientRect().top - document.querySelector('#space-channel-list').getBoundingClientRect().bottom`);
  assert.equal(directGap(), 8, 'DM divider must follow channels instead of being bottom-anchored');
  const expandedDirectTop = evaluate('pins-check', 'document.querySelector(".direct-section").getBoundingClientRect().top');
  browser('pins-check', 'click', '.channel-section-toggle');
  assert.ok(evaluate('pins-check', 'document.querySelector(".direct-section").getBoundingClientRect().top') < expandedDirectTop, 'Collapsing channels also moves DMs up');
  browser('pins-check', 'click', '.channel-section-toggle');
  assert.equal(directGap(), 8);
  assert.equal(evaluate('pins-check', '[...document.querySelectorAll(".channel-navigation button")].filter(button => button.textContent === "Browse channels").every(button => !!button.closest(".space-menu"))'), true);
  browser('pins-check', 'click', '.space-menu summary');
  screenshot('pins-check', 'pins-web-space-menu');
  action('pins-check', 'Browse channels');
  wait('pins-check', 'document.querySelector(".space-dialog[open] h2")?.textContent === "Browse channels"');
  assert.equal(evaluate('pins-check', 'document.querySelector(".space-menu").open'), false);
  browser('pins-check', 'press', 'Escape');
  wait('pins-check', '!document.querySelector(".space-dialog[open]")');
  const menu = () => {
    evaluate('pins-check', 'new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
    browser('pins-check', 'hover', `${row} .chat-message-actions-trigger`);
    wait('pins-check', `getComputedStyle(document.querySelector('${row} .chat-message-actions-trigger')).opacity === '1'`);
    browser('pins-check', 'click', `${row} .chat-message-actions-trigger`);
    wait('pins-check', '!!document.querySelector(".chat-message-actions")');
  };
  menu();
  action('pins-check', 'Pin message');
  for (const session of ['pins-check', 'pins-peer']) {
    wait(session, `document.querySelector('${row} .chat-pin-marker')?.textContent === 'Pinned by Fixture Owner'`);
    assert.equal(evaluate(session, '!!document.querySelector(".chat-message-actions")'), false, 'Pin must not require a second dialog');
    assert.equal(evaluate(session, 'document.querySelector(".chat-pins-toggle").getAttribute("aria-label")'), 'Pins, 1');
    assert.deepEqual(evaluate(session, `(() => {
      const marker = document.querySelector('${row} .chat-pin-marker');
      return [getComputedStyle(marker).color, marker.getBoundingClientRect().bottom <= document.querySelector('${row} header').getBoundingClientRect().top];
    })()`), ['rgb(228, 199, 106)', true], 'Gold attribution precedes the author for every reader');
  }
  assert.equal((await history()).pinnedMessages[0].id, message.id);
  browser('pins-check', 'hover', `${row} p`);
  screenshot('pins-check', 'pins-web-inline');
  browser('pins-check', 'click', '.chat-pins-toggle');
  wait('pins-check', '!!document.querySelector(".chat-pinned-message")');
  assert.equal(evaluate('pins-check', 'document.querySelector(".chat-pins").getAttribute("role")'), 'region');
  assert.equal(evaluate('pins-check', 'document.querySelector(".chat-pinned-message time").dateTime'), message.createdAt);
  assert.equal(evaluate('pins-check', 'document.querySelector(".chat-pinned-message p").textContent'), message.content.text);
  screenshot('pins-check', 'pins-web-list');
  action('pins-check', 'Unpin');
  wait('pins-check', 'document.querySelector(".chat-pins").textContent.includes("No pinned messages.")');
  wait('pins-peer', `!document.querySelector('${row} .chat-pin-marker')`);
  screenshot('pins-check', 'pins-web-empty');
  assert.equal((await history()).pinnedMessages.length, 0);
  await control({ incomingPin: { channelId: 'chan00000001', messageId: message.id } });
  wait('pins-peer', `document.querySelector('${row} .chat-pin-marker')?.textContent === 'Pinned by Alex'`);
  wait('pins-check', 'document.querySelector(".chat-pinned-message small")?.textContent === "Pinned by Alex"');
  await control({ incomingPin: { channelId: 'chan00000001', messageId: message.id, active: false } });
  wait('pins-check', '!document.querySelector(".chat-pinned-message")');
  browser('pins-check', 'click', '.chat-pins-toggle');
  wait('pins-check', `!!document.querySelector('${row}')`);

  // Pins and the existing reaction-details action share the same menu.
  await control({ incomingReaction: { channelId: 'chan00000001', messageId: message.id, emoji: '🚀' } });
  wait('pins-check', `!!document.querySelector('${row} .chat-reaction')`);
  menu();
  assert.equal(evaluate('pins-check', '[...document.querySelectorAll(".chat-message-actions button")].some(b => b.textContent === "Pin message")'), true);
  action('pins-check', 'View reactions');
  wait('pins-check', 'document.querySelector(".chat-reactors-list")?.textContent.includes("@alex")');
  assert.equal(evaluate('pins-check', '!!document.querySelector(".chat-message-actions")'), false);
  assert.equal((await history()).pinnedMessages.length, 0, 'Viewing reactions must not mutate pins');
  browser('pins-check', 'press', 'Escape');
  wait('pins-check', '!document.querySelector(".chat-reactors")');
  await control({ incomingReaction: { channelId: 'chan00000001', messageId: message.id, emoji: '🚀', active: false } });
  wait('pins-check', `!document.querySelector('${row} .chat-reaction')`);

  // Hold/fail a real action before it reaches the server, then retry its intent.
  evaluate('pins-check', `(() => {
    const original = window.fetch.bind(window);
    window.holdPin = true;
    window.fetch = (input, init) => window.holdPin && String(input).endsWith('/pin')
      ? new Promise(resolve => { window.failPin = () => resolve(Response.json({error:'TEST FIXTURE: pin failed'}, {status:500})); })
      : original(input, init);
  })()`);
  menu(); action('pins-check', 'Pin message');
  wait('pins-check', 'typeof window.failPin === "function"');
  menu();
  assert.equal(evaluate('pins-check', '[...document.querySelectorAll(".chat-message-actions button")].find(b => b.textContent === "Saving…")?.disabled'), true);
  assert.equal((await history()).pinnedMessages.length, 0);
  browser('pins-check', 'press', 'Escape');
  evaluate('pins-check', '(() => { window.holdPin = false; window.failPin(); })()');
  wait('pins-check', 'document.querySelector(".chat-refresh-error")?.textContent.includes("pin failed")');
  screenshot('pins-check', 'pins-web-error');
  action('pins-check', 'Retry');
  wait('pins-peer', `!!document.querySelector('${row} .chat-pin-marker')`);
  wait('pins-check', '!document.querySelector(".chat-refresh-error")');
  assert.equal((await history()).pinnedMessages.length, 1);

  browser('pins-check', 'set', 'viewport', '390', '844', '2');
  browser('pins-check', 'reload');
  wait('pins-check', `!!document.querySelector('${row}') && !document.querySelector('.chat-initial-messages')`);
  socket = new WebSocket(browser('pins-check', 'get', 'cdp-url').cdpUrl);
  await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
  });
  const cdp = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++nextId; pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const { targetInfos } = await cdp('Target.getTargets');
  const target = targetInfos.find(target => target.type === 'page' && target.url.startsWith(web));
  const { sessionId } = await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true });
  await cdp('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 1 }, sessionId);
  assert.equal(evaluate('pins-check', 'matchMedia("(pointer: coarse)").matches'), true);
  browser('pins-check', 'scrollintoview', `${row} p`);
  await delay(150);
  const point = evaluate('pins-check', `(() => { const r = document.querySelector('${row} p').getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}; })()`);
  await cdp('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [point] }, sessionId);
  await delay(650);
  await cdp('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] }, sessionId);
  wait('pins-check', '!!document.querySelector(".chat-message-actions-drawer")');
  assert.equal(evaluate('pins-check', 'document.documentElement.scrollWidth > innerWidth'), false);
  screenshot('pins-check', 'pins-web-narrow-actions');
  action('pins-check', 'Unpin message');
  wait('pins-peer', `!document.querySelector('${row} .chat-pin-marker')`);
  menu(); action('pins-check', 'Pin message');
  wait('pins-check', `!!document.querySelector('${row} .chat-pin-marker')`);
  browser('pins-check', 'click', '.chat-pins-toggle');
  wait('pins-check', '!!document.querySelector(".chat-pinned-message")');
  assert.equal(evaluate('pins-check', 'document.documentElement.scrollWidth > innerWidth'), false);
  screenshot('pins-check', 'pins-web-narrow-list');
  // Explicit DOM-only layout mock: a maximum-length name must not overlap the avatar.
  const originalName = evaluate('pins-check', 'document.querySelector(".chat-pinned-message .chat-pin-marker").lastChild.textContent');
  evaluate('pins-check', `document.querySelector('.chat-pinned-message .chat-pin-marker').lastChild.textContent = 'TEST FIXTURE '.padEnd(64, 'x')`);
  assert.deepEqual(evaluate('pins-check', `(() => {
    const article = document.querySelector('.chat-pinned-message');
    const marker = article.querySelector('.chat-pin-marker').getBoundingClientRect();
    const header = article.querySelector('header').getBoundingClientRect();
    const avatar = article.querySelector('.chat-avatar').getBoundingClientRect();
    return [marker.height > 20, marker.bottom <= header.top, avatar.top === header.top, document.documentElement.scrollWidth <= innerWidth];
  })()`), [true, true, true, true], 'Wrapping pinner names preserve author/avatar alignment');
  screenshot('pins-check', 'pins-web-narrow-long-name-mock');
  evaluate('pins-check', `document.querySelector('.chat-pinned-message .chat-pin-marker').lastChild.textContent = ${JSON.stringify(originalName)}`);
  socket.close(); socket = undefined;

  // A newly opened channel finds a pin older than its 50-message history page.
  const old = initial.messages[0];
  await control({ incomingPin: { channelId: 'chan00000001', messageId: old.id } });
  for (let i = 0; i < 55; i++) await control({ incomingMessage: { channelId: 'chan00000001', text: `Pagination ${i}` } });
  const page = await history();
  assert.equal(page.messages.length, 50);
  assert.equal(page.messages.some(m => m.id === old.id), false);
  assert.ok(page.pinnedMessages.some(m => m.id === old.id));
  browser('pins-check', 'reload');
  wait('pins-check', 'document.querySelector(".chat-pins-toggle")?.getAttribute("aria-label") === "Pins, 2"');
  browser('pins-check', 'click', '.chat-pins-toggle');
  wait('pins-check', `document.querySelector('.chat-pins')?.textContent.includes(${JSON.stringify(old.content.text)})`);
  browser('pins-check', 'set', 'viewport', '1280', '900', '2');
  action('pins-check', 'design Voice session duration');
  wait('pins-check', 'document.querySelector("#chat-heading")?.textContent === "# design"');
  assert.equal(evaluate('pins-check', '!!document.querySelector(".chat-pins")'), false, 'Channel changes must leave the previous pin view');
  assert.equal(evaluate('pins-check', 'document.querySelector(".chat-pins-toggle").getAttribute("aria-label")'), 'Pins, 0');
  console.log('PASS: compact desktop hover/focus controls, gold top attribution, content-following DMs, space-menu Browse, one-action pin/unpin, two-tab fanout, full pin history, reaction-details coexistence, pending/error/retry, touch drawer, narrow/wrapping layout and channel isolation.');
} catch (error) {
  console.error(browser('pins-check', 'snapshot', '-i').snapshot);
  browser('pins-check', 'screenshot', '/tmp/caper-pins-test-failure.png');
  throw error;
} finally {
  socket?.close();
  for (const session of ['pins-check', 'pins-peer']) {
    try { browser(session, 'close'); } catch { /* Preserve the original assertion failure. */ }
  }
}
