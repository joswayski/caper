// Real homepage UI, local demo state only. No live auth or chat requests are sent.
// Run against local Vite: node scripts/test-home-demo.mjs http://localhost:30701
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

const origin = new URL(process.argv[2] ?? 'http://localhost:5174');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname), 'Local development only');
const session = `home-${process.pid}`;
function browser(...args) {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', session, ...args, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = code => browser('eval', `(async () => { ${code} })()`).result;
const wait = code => browser('wait', '--fn', `Boolean(${code})`);
const nextMoments = count => evaluate(`for (let i = 0; i < ${count}; i++) { document.querySelector('[aria-label="Next demo moment"]').click(); await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))); }`);
const members = () => evaluate('return [...document.querySelectorAll(".sim-member strong")].map(el => el.textContent)');
const voices = () => evaluate('return [...document.querySelectorAll(".sim-person strong")].map(el => el.textContent)');

try {
  browser('open', origin.href);
  browser('network', 'route', '**/api/account/me', '--body', 'null');
  browser('set', 'viewport', '1280', '800', '2');
  browser('set', 'media', 'dark', 'reduced-motion');
  browser('reload');
  wait('document.querySelector(".sim-footer button:disabled")');
  const before = evaluate('return document.querySelector(".sim-messages").textContent');
  browser('wait', '2500');
  assert.equal(evaluate('return document.querySelector(".sim-messages").textContent'), before, 'Reduced motion freezes the script');
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".live-activator")).placeItems'), 'center');
  assert.equal(evaluate('return document.querySelector(".live-activator").getAttribute("href")'), '/spaces');

  // Production surfaces from shared/design.css; don't derive expectations from the demo.
  assert.deepEqual(evaluate('return [".sim-sidebar", ".sim-chat", ".sim-composer"].map(s => getComputedStyle(document.querySelector(s)).backgroundColor)'), ['rgb(21, 28, 30)', 'rgb(25, 33, 35)', 'rgb(40, 49, 51)']);
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".sim-avatar")).borderRadius'), '30%');
  assert.equal(evaluate('return document.querySelector(".sim-composer").getAttribute("href")'), '/spaces');
  assert.notEqual(evaluate('return getComputedStyle(document.querySelector(".sim-members")).display'), 'none');

  const chip = '.sim-message:last-of-type .sim-reactions button[aria-pressed]';
  const count = evaluate(`return Number(document.querySelector('${chip} span').textContent)`);
  assert.ok(evaluate(`const el = document.querySelector('${chip}'); const r = el.getBoundingClientRect(); return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest('button') === el;`), '3D panel must not intercept reaction hit testing');
  browser('click', chip);
  assert.equal(evaluate(`return document.querySelector('${chip}').getAttribute('aria-pressed')`), 'true');
  assert.equal(evaluate(`return Number(document.querySelector('${chip} span').textContent)`), count + 1);
  browser('click', chip);
  assert.equal(evaluate(`return Number(document.querySelector('${chip} span').textContent)`), count);

  browser('click', '.sim-message:last-of-type .sim-add-reaction');
  wait('document.querySelector(".sim-emoji-picker")');
  browser('click', '.sim-emoji-picker button:last-child');
  assert.equal(evaluate('return document.querySelectorAll(".sim-message:last-of-type button[aria-pressed=true]").length'), 1);
  browser('click', '.sim-message:last-of-type .sim-add-reaction');
  browser('press', 'Escape');
  assert.equal(evaluate('return !!document.querySelector(".sim-emoji-picker")'), false);
  assert.ok(evaluate('return document.activeElement.matches(".sim-add-reaction")'));

  // Advance with the public control, even under reduced motion. Joining a
  // channel must not automatically join voice; leaving voice keeps membership.
  assert.deepEqual(members(), ['Maya', 'Theo', 'June']);
  browser('click', '[aria-label="Next demo moment"]'); // 4
  assert.deepEqual(members(), ['Maya', 'Theo', 'June', 'Leo']);
  assert.deepEqual(voices(), ['Maya', 'Theo', 'June']);
  assert.equal(evaluate('return document.querySelector(".sim-arrival").textContent'), 'Leo joined the channel');
  nextMoments(4); // 8
  assert.ok(members().includes('Noor'));
  assert.ok(!voices().includes('Noor'));
  assert.equal(evaluate('return document.querySelector(".sim-typing").textContent.trim()'), 'Noor and Theo are typing');
  nextMoments(2); // 10
  assert.ok(voices().includes('Leo'));
  nextMoments(5); // 15
  assert.equal(members().length, 6);
  assert.ok(!voices().includes('Sam'));
  nextMoments(7); // 22
  assert.ok(members().includes('Maya'));
  assert.ok(!voices().includes('Maya'));
  nextMoments(16); // 38
  assert.equal(voices().length, 6);
  nextMoments(1); // 39
  assert.ok(!members().includes('June'));
  assert.ok(!voices().includes('June'));
  nextMoments(6); // 45
  assert.equal(members().length, 6);
  assert.equal(voices().length, 6);
  assert.ok(evaluate('return document.querySelector(".sim-messages").textContent.includes("back with cookies. let’s gooo")'));
  nextMoments(4); // 0: reset presence as well as messages
  assert.deepEqual(members(), ['Maya', 'Theo', 'June']);
  assert.deepEqual(voices(), ['Maya', 'Theo', 'June']);
  console.log('PASS: six-person join/leave/rejoin sequence, separate voice membership, multi-person typing, manual stepping and cycle reset');

  // Mock only clipboard rejection: browsers may deny it, and we must not claim success.
  evaluate('navigator.clipboard.writeText = async () => { throw new Error("Test denial"); };');
  browser('click', 'button.contact-action');
  wait('document.querySelector(".contact-feedback").textContent.includes("Couldn’t copy")');
  assert.ok(evaluate('return document.querySelector(".contact-feedback").textContent.includes("contact@josevalerio.com")'));
  assert.deepEqual(evaluate('return performance.getEntriesByType("resource").filter(r => /\\/api\\/(chat|media|channels)/.test(r.name)).map(r => r.name)'), []);
  browser('click', '.live-invite');
  wait('document.querySelector("#email")');
  assert.equal(evaluate('return location.pathname'), '/login');
  const login = evaluate('return document.querySelector("main").textContent');
  assert.ok(login.includes('We’ll send a code to your email.'));
  assert.ok(!/WELCOME TO CAPER|We only send a code when you ask|No password needed/.test(login));
  browser('set', 'viewport', '390', '844', '2');
  browser('open', origin.href);
  wait('document.querySelector(".live-stage[data-ready]")');
  evaluate('await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));');
  const gap = evaluate('return document.querySelector(".live-scene").getBoundingClientRect().top - document.querySelector(".hero-lede").getBoundingClientRect().bottom');
  assert.ok(gap >= 40, `Mobile demo needs clearance above its projected edge; got ${gap}px`);
  // Local account fixtures: a returning member must never visit the login route.
  browser('network', 'unroute', '**/api/account/me');
  browser('network', 'route', '**/api/account/me', '--body', JSON.stringify({ id: 'demo-test', username: 'demo-test', displayName: 'Demo test' }));
  browser('network', 'route', '**/api/spaces', '--body', JSON.stringify({ spaces: [], limits: { ownedSpaces: 10, totalSpaces: 10, channelsPerSpace: 10 } }));
  browser('network', 'route', '**/login', '--body', '<h1>Unexpected login navigation</h1>');
  browser('click', '.live-invite');
  wait('document.querySelector(".spaces-empty")');
  assert.equal(evaluate('return location.pathname'), '/spaces');
  assert.equal(evaluate('return !!document.querySelector("#email")'), false);
  browser('network', 'unroute', '**/api/account/me');
  browser('network', 'route', '**/api/account/me', '--body', JSON.stringify({ id: 'demo-test', username: null, displayName: null }));
  browser('open', origin.href);
  wait('document.querySelector(".live-stage[data-ready]")');
  browser('click', '.live-invite');
  wait('location.pathname === "/profile"');
  console.log('PASS: centered CTA, reduced motion, 3D reaction hit testing, count toggles, picker/Escape, clipboard failure, no chat/media requests, email login');
  console.log('PASS: simplified login copy and mobile demo clearance');
  console.log('PASS: signed-in join bypasses login; incomplete accounts reach profile setup (mocked accounts)');
} finally {
  browser('close');
}
