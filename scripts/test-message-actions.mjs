// Disposable loopback fixture + Chromium touch input, not a physical phone test.
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
const browser = (...command) => {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', 'message-actions', ...command, '--json'], { encoding: 'utf8', timeout: 60_000 }));
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const screenshot = name => {
  if (!artifacts) return;
  evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))');
  browser('screenshot', `${artifacts}/${name}.png`);
};
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  assert.equal(response.status, 200);
}
async function history() {
  return (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers: { authorization: 'Bearer fixture-owner-token' } })).json();
}

let socket;
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  await control({ reset: true });
  browser('open', 'about:blank');
  browser('set', 'viewport', '390', '844', '2');
  browser('cookies', 'set', 'caper_fixture', 'owner', '--url', web, '--path', '/', '--sameSite', 'Lax');
  browser('open', `${web}/spaces`);
  wait('!!document.querySelector(".chat-message") && !document.querySelector(".chat-initial-messages")');

  // agent-browser has no touch command. Use its launched Chromium's CDP input
  // rather than synthesizing DOM events or installing another browser package.
  socket = new WebSocket(browser('get', 'cdp-url').cdpUrl);
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
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  await cdp('Browser.grantPermissions', { origin: web, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'] });
  const { targetInfos } = await cdp('Target.getTargets');
  const target = targetInfos.find(target => target.type === 'page' && target.url.startsWith(web));
  assert.ok(target);
  const { sessionId } = await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true });
  await cdp('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 2 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  const touch = (type, x, y) => cdp('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' || type === 'touchCancel' ? [] : [{ x, y }] }, sessionId);
  const point = selector => evaluate(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  const hold = async (selector = '.chat-message p', duration = 650) => {
    browser('scrollintoview', selector);
    await delay(150);
    const { x, y } = point(selector);
    assert.ok(y > 0 && y < 844, `Touch target must be in the viewport, got y=${y}`);
    await touch('touchStart', x, y);
    await delay(duration);
    await touch('touchEnd');
  };
  const open = async () => {
    const key = evaluate('[...document.querySelectorAll(".chat-message")].at(-1).dataset.messageKey');
    await hold(`[data-message-key="${key}"] p`);
    wait('!!document.querySelector(".chat-message-actions")');
  };
  const action = name => browser('find', 'role', 'button', 'click', '--name', name, '--exact');

  // A tap, a scroll gesture and cancellation must not open a long-press menu.
  await hold('.chat-message p', 100);
  await delay(550);
  assert.equal(evaluate('!!document.querySelector(".chat-message-actions")'), false);
  const first = point('.chat-message p');
  await touch('touchStart', first.x, first.y);
  await touch('touchMove', first.x, first.y + 40);
  await delay(650);
  await touch('touchEnd');
  assert.equal(evaluate('!!document.querySelector(".chat-message-actions")'), false);
  await touch('touchStart', first.x, first.y);
  await touch('touchCancel');
  await delay(650);
  assert.equal(evaluate('!!document.querySelector(".chat-message-actions")'), false);

  const text = 'TEST FIXTURE — First line 🚀\n  Second line with  two spaces.';
  await control({ incomingMessage: { channelId: 'chan00000001', text: 'First line 🚀\n  Second line with  two spaces.' } });
  wait('document.querySelector(".chat-messages").textContent.includes("Second line with  two spaces.")');
  const message = (await history()).messages.at(-1);
  assert.notEqual(message.id, message.clientMessageId);
  assert.notEqual(message.id, message.seq);
  // Find the target by its retry UUID only as a selector; clipboard expectations
  // come from the fixture's independent API response and literal text above.
  const row = `[data-message-key="${message.clientMessageId}"]`;
  const openTarget = async () => { await hold(`${row} p`); wait('!!document.querySelector(".chat-message-actions")'); };
  assert.equal(evaluate('[...document.querySelectorAll(".chat-add-reaction")].every(b => getComputedStyle(b).display === "none")'), true);
  screenshot('message-actions-mobile-conversation');
  await openTarget();
  assert.deepEqual((await history()).messages.at(-1).reactions ?? [], [], 'Releasing a hold must not click a quick reaction under the finger');
  assert.equal(evaluate('document.querySelector(".chat-message-actions").getAttribute("role")'), 'dialog');
  assert.equal(evaluate('document.querySelectorAll(".chat-quick-reactions button").length'), 6);
  assert.equal(evaluate('document.documentElement.scrollWidth > innerWidth'), false);
  browser('press', 'Tab');
  assert.equal(evaluate('document.querySelector(".chat-message-actions").contains(document.activeElement)'), true);
  screenshot('message-actions-mobile-drawer');
  action('Copy text');
  wait('!document.querySelector(".chat-message-actions")');
  assert.equal(browser('clipboard', 'read').text, text);
  await openTarget();
  action('Copy message ID');
  wait('!document.querySelector(".chat-message-actions")');
  assert.equal(browser('clipboard', 'read').text, message.id);

  await openTarget();
  action('React with 👍');
  wait(`document.querySelector(${JSON.stringify(row + ' .chat-reaction')})?.getAttribute('aria-pressed') === 'true' && !document.querySelector(${JSON.stringify(row + ' .chat-reaction')}).disabled`);
  assert.deepEqual((await history()).messages.at(-1).reactions, [{ emoji: '👍', authorIds: ['owner0000001'] }]);
  await openTarget();
  assert.deepEqual((await history()).messages.at(-1).reactions, [{ emoji: '👍', authorIds: ['owner0000001'] }], 'Opening the drawer must preserve the existing reaction snapshot');
  assert.equal(evaluate('document.querySelector(".chat-quick-reactions button").getAttribute("aria-pressed")'), 'true');
  action('React with 👍');
  wait(`!document.querySelector(${JSON.stringify(row + ' .chat-reaction')})`);
  assert.deepEqual((await history()).messages.at(-1).reactions, []);

  await openTarget();
  action('Add reaction');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  screenshot('message-actions-mobile-picker');
  const emojiBody = '.chat-reaction-picker .epr-body';
  assert.equal(evaluate(`(() => { const body = document.querySelector(${JSON.stringify(emojiBody)}); return body.scrollHeight > body.clientHeight; })()`), true);
  const grid = point(emojiBody);
  await touch('touchStart', grid.x, grid.y);
  for (let step = 1; step <= 5; step++) {
    await touch('touchMove', grid.x, grid.y - step * 30);
    await delay(30);
  }
  await touch('touchEnd');
  wait(`document.querySelector(${JSON.stringify(emojiBody)}).scrollTop > 0`);
  browser('fill', '.chat-reaction-picker input', 'definitely-no-such-emoji');
  wait('document.querySelector(".epr-status-search-results")?.textContent.includes("No")');
  screenshot('message-actions-mobile-empty-search');
  browser('fill', '.chat-reaction-picker input', 'rocket');
  // Live arrivals must not retarget or unmount the picker when its row scrolls out.
  for (let index = 0; index < 12; index++) await control({ incomingMessage: { channelId: 'chan00000001', text: `live arrival ${index}` } });
  wait('document.querySelector(".chat-messages").textContent.includes("live arrival 11")');
  assert.equal(evaluate('document.querySelector(".chat-reaction-picker input").value'), 'rocket');
  browser('click', '.chat-reaction-picker button[data-unified="1f680"]');
  wait('!document.querySelector(".chat-reaction-picker")');
  // Closing the picker precedes the asynchronous save; its row is off-screen.
  let saved;
  for (let attempt = 0; attempt < 100; attempt++) {
    saved = (await history()).messages.find(item => item.id === message.id);
    if (saved?.reactions?.some(reaction => reaction.emoji === '🚀')) break;
    await delay(100);
  }
  assert.deepEqual(saved?.reactions, [{ emoji: '🚀', authorIds: ['owner0000001'] }]);
  assert.deepEqual((await history()).messages.at(-1).reactions ?? [], []);

  // Keep the next target in view after the live-scroll check.
  await open();
  evaluate('window.originalClipboardWrite = navigator.clipboard.writeText.bind(navigator.clipboard); navigator.clipboard.writeText = () => Promise.reject(new Error("TEST FIXTURE: clipboard denied"))');
  action('Copy text');
  wait('!!document.querySelector(".chat-action-error")');
  screenshot('message-actions-mobile-copy-error');
  evaluate('navigator.clipboard.writeText = window.originalClipboardWrite');
  browser('press', 'Escape');
  wait('!document.querySelector(".chat-message-actions")');
  await open();
  browser('click', '.chat-actions-overlay');
  wait('!document.querySelector(".chat-message-actions")');

  const left = await fetch(`${api}/api/spaces/space0000001/channels/chan00000002/membership`, { method: 'DELETE', headers: { authorization: 'Bearer fixture-owner-token' } });
  assert.equal(left.status, 204);
  browser('reload');
  wait('!!document.querySelector(".chat-message") && !document.querySelector(".chat-initial-messages")');
  browser('click', '.navigation-toggle');
  browser('click', '.browse-channels');
  wait('!!document.querySelector(".channel-directory")');
  action('Preview #design');
  wait('!!document.querySelector(".channel-preview") && !document.querySelector(".chat-initial-messages")');
  await open();
  assert.equal(evaluate('!!document.querySelector(".chat-quick-reactions")'), false);
  assert.equal(evaluate('document.querySelectorAll(".chat-copy-actions button").length'), 2);
  screenshot('message-actions-mobile-read-only');
  action('Copy message ID');
  wait('!document.querySelector(".chat-message-actions")');
  assert.ok(browser('clipboard', 'read').text.startsWith('message-chan00000002-'));

  await cdp('Emulation.setTouchEmulationEnabled', { enabled: false }, sessionId);
  browser('set', 'viewport', '1280', '900', '2');
  action('Join channel');
  wait('!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-add-reaction:not(:disabled)")');
  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  wait('document.querySelector(".chat-reaction-picker").getBoundingClientRect().right > innerWidth - 80');
  assert.equal(evaluate('!!document.querySelector(".chat-actions-overlay")'), false);
  screenshot('message-actions-desktop-picker');
  browser('press', 'Escape');
  wait('!document.querySelector(".chat-reaction-picker")');
  console.log('PASS: Chromium touch gestures (tap/scroll/cancel/hold), hidden inline emoji controls, modal focus/dismissal, exact clipboard text/ID, quick toggles, picker search/live-target retention, clipboard failure, read-only copying, desktop picker.');
} catch (error) {
  browser('screenshot', '/tmp/message-actions-failure.png');
  throw error;
} finally {
  socket?.close();
  browser('close');
}
