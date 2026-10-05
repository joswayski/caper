// Disposable loopback fixture + Chromium touch input, not a physical phone test.
// Start native-parity-fixture.mjs and Vite before running this script.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { emojiAsset } from '../apps/web/src/chat/emoji.ts';

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
const visibleEmojiImages = `(() => {
  const body = document.querySelector('.epr-body').getBoundingClientRect();
  return [...document.querySelectorAll('.epr-body img')].filter(image => {
    const rect = image.getBoundingClientRect();
    return rect.bottom > body.top && rect.top < body.bottom;
  });
})()`;
function checkEmojiCategories(layout) {
  assert.equal(evaluate(`document.querySelector('.epr-category-nav').getBoundingClientRect().bottom <= document.querySelector('.epr-search-container').getBoundingClientRect().top`), true);
  for (const [category, firstEmoji] of [['activities', '1f383'], ['flags', '1f3c1'], ['smileys_people', '1f600']]) {
    const tab = `.epr-icn-${category}`;
    browser('click', tab);
    wait(`document.querySelector('${tab}').getAttribute('aria-selected') === 'true' && !!document.querySelector('.epr-body button[data-unified="${firstEmoji}"]')`);
    wait(`${visibleEmojiImages}.length >= 24 && ${visibleEmojiImages}.every(image => image.complete && image.naturalWidth > 0)`);
    assert.equal(evaluate(`${visibleEmojiImages}.every(image => image.loading === 'eager' && new URL(image.src).pathname.startsWith('/emoji/twemoji-15/'))`), true, 'Virtualized images must load eagerly from local artwork');
    assert.equal(evaluate(`getComputedStyle(document.querySelector('${tab}'), '::before').content`), 'none', 'Clicking a category must not show the circular focus ring');
    assert.deepEqual(evaluate(`(() => { const tab = document.querySelector('${tab}'); const underline = getComputedStyle(tab, '::after'); return [getComputedStyle(tab).color, underline.height, underline.backgroundColor]; })()`), ['rgb(243, 244, 245)', '2px', 'rgb(182, 77, 50)']);
    screenshot(`emoji-${layout}-${category}`);
  }
  assert.ok(evaluate('document.querySelectorAll(".epr-body img").length') < 200, 'Eager loading must retain the virtualized window, not mount the whole catalog');
}
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  assert.equal(response.status, 200);
}
async function history() {
  return (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers: { authorization: 'Bearer fixture-owner-token' } })).json();
}

function holdReactions() {
  // Hold before fixture delivery: server pushes cannot make a pessimistic
  // client pass the immediate-render checks.
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.heldReactions = [];
    window.holdReactions = true;
    window.fetch = (input, init) => window.holdReactions && String(input).endsWith('/reactions')
      ? new Promise(resolve => window.heldReactions.push(status => resolve(status
        ? Response.json({ error: 'TEST FIXTURE: reaction failed' }, { status }) : original(input, init))))
      : original(input, init);
  })()`);
}

let socket;
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  await control({ reset: true });
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  browser('cookies', 'set', 'caper_fixture', 'owner', '--url', web, '--path', '/', '--sameSite', 'Lax');
  browser('open', `${web}/spaces`);
  wait('!!document.querySelector(".chat-message") && !document.querySelector(".chat-initial-messages")');
  wait('performance.getEntriesByType("resource").some(resource => resource.name.includes("ReactionPicker"))');
  assert.equal(evaluate('!!document.querySelector(".chat-reaction-picker")'), false);
  assert.equal(evaluate('performance.getEntriesByType("resource").filter(resource => resource.name.includes("/emoji/twemoji-15/")).length'), 0, 'Warming picker code must not fetch the image catalog');
  const preloadedImages = `performance.getEntriesByType('resource').filter(resource => resource.name.includes('/emoji/twemoji-15/') && resource.name.endsWith('.svg'))`;
  for (const intent of ['hover', 'focus']) {
    if (intent === 'focus') {
      browser('reload');
      wait('!!document.querySelector(".chat-message") && !document.querySelector(".chat-initial-messages")');
    }
    assert.equal(evaluate(`${preloadedImages}.length`), 0);
    // Vite module requests can fill Chromium's default 250-entry timing buffer.
    evaluate('(() => { performance.setResourceTimingBufferSize(2000); performance.clearResourceTimings(); })()');
    const messageKey = evaluate('document.querySelector(".chat-message").dataset.messageKey');
    const trigger = `[data-message-key="${messageKey}"] .chat-add-reaction`;
    browser(intent, trigger);
    wait(`${preloadedImages}.length === 128 && ${preloadedImages}.every(resource => resource.responseEnd > 0 && resource.responseStatus === 200)`);
    assert.equal(evaluate(`${preloadedImages}.some(resource => resource.name.endsWith('/1f600.svg'))`), true);
    assert.equal(evaluate('!!document.querySelector(".chat-reaction-picker")'), false, `${intent} must preload images without opening the picker`);
    browser('mouse', 'move', '10', '10');
    browser('focus', '.chat-composer textarea');
    browser(intent, trigger);
    await delay(100);
    assert.equal(evaluate(`${preloadedImages}.length`), 128, 'Repeated intent must not issue duplicate image requests');
  }

  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  wait(`${visibleEmojiImages}.length >= 24 && ${visibleEmojiImages}.every(image => image.complete && image.naturalWidth > 0)`);
  // The library itself warms some off-screen assets. Chromium can reuse them
  // without a new Resource Timing entry, so count only genuinely cold assets.
  const loadedAssets = new Set(evaluate(`${preloadedImages}.map(resource => new URL(resource.name).pathname)`));
  for (const [category, firstEmoji, count, intent] of [
    ['travel_places', '1f30d', 128, 'hover'], ['flags', '1f3c1', 128, 'focus'],
    ['animals_nature', '1f435', 128, 'hover'], ['food_drink', '1f347', 128, 'focus'],
    ['activities', '1f383', 85, 'hover'], ['objects', '1f453', 128, 'focus'],
    ['symbols', '1f3e7', 128, 'hover'],
  ]) {
    const response = await fetch(`${web}/emoji/twemoji-15/preload-${category}.json`);
    assert.equal(response.status, 200);
    const codes = await response.json();
    assert.equal(codes.length, count);
    assert.equal(codes[0], firstEmoji);
    const assets = codes.map(emojiAsset);
    const coldAssets = assets.filter(asset => !loadedAssets.has(asset));
    evaluate('performance.clearResourceTimings()');
    // Hover the nested SVG, not just the button, to exercise target resolution.
    browser(intent, `.epr-icn-${category}${intent === 'hover' ? ' svg' : ''}`);
    wait(`${JSON.stringify(coldAssets)}.every(asset => ${preloadedImages}.some(resource => new URL(resource.name).pathname === asset && resource.responseEnd > 0 && resource.responseStatus === 200))`);
    const requestedAssets = evaluate(`${preloadedImages}.map(resource => new URL(resource.name).pathname)`);
    assert.ok(requestedAssets.length <= count && requestedAssets.every(asset => assets.includes(asset)), 'Category intent must fetch only its bounded manifest');
    requestedAssets.forEach(asset => loadedAssets.add(asset));
    assert.ok(assets.every(asset => loadedAssets.has(asset)), 'Every prefetched asset must be loaded before selection');
    const manifests = `performance.getEntriesByType('resource').filter(resource => resource.name.includes('/emoji/twemoji-15/preload-') && resource.name.endsWith('.json'))`;
    assert.deepEqual(evaluate(`${manifests}.map(resource => new URL(resource.name).pathname)`), [`/emoji/twemoji-15/preload-${category}.json`]);
    assert.equal(evaluate(`document.querySelector('.epr-cat-btn[aria-selected="true"]').classList.contains('epr-icn-smileys_people')`), true, 'Category intent must not select or scroll away from smileys');
    assert.equal(evaluate('document.querySelector(".epr-body").scrollTop'), 0);
    browser('mouse', 'move', '10', '10');
    browser('focus', '.chat-reaction-picker input');
    browser(intent, `.epr-icn-${category}`);
    await delay(100);
    assert.equal(evaluate(`${preloadedImages}.length`), requestedAssets.length, 'Repeated category intent must reuse loaded images');
    assert.equal(evaluate(`${manifests}.length`), 1, 'Repeated category intent must reuse its manifest');
  }
  evaluate('performance.clearResourceTimings()');
  browser('hover', '.chat-reaction-picker input');
  browser('focus', '.chat-reaction-picker input');
  await delay(100);
  assert.equal(evaluate(`${preloadedImages}.length`), 0, 'Unrelated picker controls must not preload categories');
  browser('click', '.epr-icn-travel_places');
  wait(`document.querySelector('.epr-icn-travel_places').getAttribute('aria-selected') === 'true' && !!document.querySelector('.epr-body button[data-unified="1f30d"]')`);
  wait(`${visibleEmojiImages}.length >= 24 && ${visibleEmojiImages}.every(image => image.complete && image.naturalWidth > 0)`);
  screenshot('emoji-desktop-prefetched-travel');
  browser('press', 'Escape');
  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  wait(`${visibleEmojiImages}.length >= 24 && ${visibleEmojiImages}.every(image => image.complete && image.naturalWidth > 0)`);
  evaluate('performance.clearResourceTimings()');
  browser('hover', '.epr-icn-travel_places svg');
  browser('focus', '.epr-icn-flags');
  await delay(100);
  assert.equal(evaluate(`performance.getEntriesByType('resource').filter(resource => resource.name.includes('/emoji/twemoji-15/')).length`), 0, 'Reopening must preserve completed category preloads');
  browser('press', 'Escape');

  browser('set', 'viewport', '390', '844', '2');
  // Initialize the narrow layout instead of carrying the desktop members pane
  // into its mobile overlay while checking unrelated touch gestures.
  browser('reload');
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

  holdReactions();
  await openTarget();
  action('React with 👍');
  wait(`document.querySelector(${JSON.stringify(row + ' .chat-reaction')})?.getAttribute('aria-pressed') === 'true' && !document.querySelector(${JSON.stringify(row + ' .chat-reaction')}).disabled`);
  assert.deepEqual((await history()).messages.at(-1).reactions ?? [], [], 'Own chip must appear before the server sees the write');
  assert.equal(evaluate('document.body.textContent.includes("Saving reaction")'), false);
  browser('scrollintoview', row + ' .chat-reaction');
  screenshot('reaction-mobile-optimistic');
  await openTarget();
  assert.equal(evaluate('document.querySelector(".chat-quick-reactions button").getAttribute("aria-pressed")'), 'true');
  action('React with 👍');
  wait(`!document.querySelector(${JSON.stringify(row + ' .chat-reaction')})`);
  assert.equal(evaluate('window.heldReactions.length'), 1, 'Rapid toggles serialize writes while updating instantly');
  evaluate('window.heldReactions[0]()');
  wait('window.heldReactions.length === 2');
  assert.equal(evaluate(`!!document.querySelector(${JSON.stringify(row + ' .chat-reaction')})`), false, 'The older add acknowledgement must not flash the removed chip');
  evaluate('window.heldReactions[1]()');
  for (let attempt = 0; attempt < 100 && (await history()).messages.at(-1).reactions?.length; attempt++) await delay(50);
  assert.deepEqual((await history()).messages.at(-1).reactions, []);

  await openTarget();
  action('React with 👍');
  wait('window.heldReactions.length === 3');
  evaluate('window.heldReactions[2](503)');
  wait(`!!document.querySelector(${JSON.stringify(row + ' [role="alert"]')}) && !document.querySelector(${JSON.stringify(row + ' .chat-reaction')})`);
  browser('scrollintoview', row + ' .chat-send-error');
  screenshot('reaction-mobile-error');
  browser('click', row + ' .chat-send-error button:first-of-type');
  wait(`window.heldReactions.length === 4 && !document.querySelector(${JSON.stringify(row + ' [role="alert"]')}) && !!document.querySelector(${JSON.stringify(row + ' .chat-reaction[aria-pressed="true"]')})`);
  evaluate('window.holdReactions = false; window.heldReactions[3]()');
  for (let attempt = 0; attempt < 100 && !(await history()).messages.at(-1).reactions?.length; attempt++) await delay(50);
  assert.deepEqual((await history()).messages.at(-1).reactions, [{ emoji: '👍', authorIds: ['owner0000001'] }]);
  browser('scrollintoview', row + ' .chat-reaction');
  browser('click', row + ' .chat-reaction');
  for (let attempt = 0; attempt < 100 && (await history()).messages.at(-1).reactions?.length; attempt++) await delay(50);

  await openTarget();
  action('Add reaction');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  checkEmojiCategories('mobile');
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
  assert.ok(browser('clipboard', 'read').text.startsWith('chan00000002m'));

  await cdp('Emulation.setTouchEmulationEnabled', { enabled: false }, sessionId);
  browser('set', 'viewport', '1280', '900', '2');
  action('Join channel');
  wait('!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-add-reaction:not(:disabled)")');
  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  wait('document.querySelector(".chat-reaction-picker").getBoundingClientRect().right > innerWidth - 80');
  assert.equal(evaluate('!!document.querySelector(".chat-actions-overlay")'), false);
  checkEmojiCategories('desktop');
  browser('fill', '.chat-reaction-picker input', '');
  browser('focus', '.chat-reaction-picker input');
  browser('press', 'Tab');
  wait('document.activeElement?.classList.contains("epr-cat-btn")');
  assert.equal(evaluate('document.activeElement.matches(":focus-visible") && getComputedStyle(document.activeElement).outlineStyle === "solid"'), true, 'Category tabs must retain keyboard focus indication');
  screenshot('emoji-desktop-keyboard-focus');
  const focusedCategory = evaluate('document.activeElement.getAttribute("aria-label")');
  browser('press', 'Enter');
  wait(`document.querySelector('.epr-cat-btn[aria-selected="true"]').getAttribute('aria-label') === ${JSON.stringify(focusedCategory)}`);
  screenshot('message-actions-desktop-picker');
  browser('press', 'Escape');
  wait('!document.querySelector(".chat-reaction-picker")');
  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  browser('click', '.epr-icn-activities');
  wait(`document.querySelector('.epr-icn-activities').getAttribute('aria-selected') === 'true' && !!document.querySelector('.epr-body button[data-unified="1f383"]')`);
  wait(`${visibleEmojiImages}.length >= 24 && ${visibleEmojiImages}.every(image => image.complete && image.naturalWidth > 0)`);
  screenshot('emoji-desktop-reopened');
  browser('press', 'Escape');
  holdReactions();
  browser('find', 'first', '.chat-add-reaction:not(:disabled)', 'click');
  wait('!!document.querySelector(".chat-reaction-picker input")');
  browser('fill', '.chat-reaction-picker input', 'rocket');
  wait(`!!document.querySelector('.chat-reaction-picker button[data-unified="1f680"]')`);
  browser('click', '.chat-reaction-picker button[data-unified="1f680"]');
  wait(`!!document.querySelector('.chat-reaction[aria-pressed="true"]:not(:disabled)')`);
  assert.equal(evaluate('document.body.textContent.includes("Saving reaction")'), false);
  screenshot('reaction-desktop-optimistic');
  evaluate('window.holdReactions = false; window.heldReactions.at(-1)()');
  console.log('PASS: Chromium touch gestures (tap/scroll/cancel/hold), hidden inline emoji controls, modal focus/dismissal, exact clipboard text/ID, quick toggles, picker code warmup without artwork fetches, trigger and all seven category hover/focus preloads without duplicate requests or selection, category cache across reopen, eager virtualized category jumps/reopen, underline/no click ring, keyboard tabs, search/live-target retention, clipboard failure, read-only copying, desktop picker.');
  console.log('PASS: optimistic mobile/desktop reactions before request delivery, enabled chips, no saving status, rapid add/remove without stale-ack flicker, error rollback and immediate retry.');
} catch (error) {
  console.error('Emoji requests at failure:', evaluate(`(() => {
    const resources = performance.getEntriesByType('resource').filter(resource => resource.name.includes('/emoji/twemoji-15/'));
    return { images: resources.filter(resource => resource.name.endsWith('.svg')).length, manifests: resources.filter(resource => resource.name.endsWith('.json')).map(resource => new URL(resource.name).pathname) };
  })()`));
  browser('screenshot', '/tmp/message-actions-failure.png');
  throw error;
} finally {
  socket?.close();
  browser('close');
}
