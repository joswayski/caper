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

try {
  browser('open', origin.href);
  browser('set', 'viewport', '1280', '800', '2');
  browser('set', 'media', 'dark', 'reduced-motion');
  browser('reload');
  wait('document.querySelector(".sim-footer button")?.disabled');
  const before = evaluate('return document.querySelector(".sim-messages").textContent');
  browser('wait', '2500');
  assert.equal(evaluate('return document.querySelector(".sim-messages").textContent'), before, 'Reduced motion freezes the script');
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".live-activator")).placeItems'), 'center');
  assert.equal(evaluate('return document.querySelector(".live-activator").getAttribute("href")'), '/login');

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

  // Mock only clipboard rejection: browsers may deny it, and we must not claim success.
  evaluate('navigator.clipboard.writeText = async () => { throw new Error("Test denial"); };');
  browser('click', 'button.contact-action');
  wait('document.querySelector(".contact-feedback").textContent.includes("Couldn’t copy")');
  assert.ok(evaluate('return document.querySelector(".contact-feedback").textContent.includes("contact@josevalerio.com")'));
  assert.deepEqual(evaluate('return performance.getEntriesByType("resource").filter(r => /\\/api\\/(chat|media|channels)/.test(r.name)).map(r => r.name)'), []);
  browser('click', '.live-invite');
  wait('document.querySelector("#email")');
  assert.equal(evaluate('return location.pathname'), '/login');
  console.log('PASS: centered CTA, reduced motion, 3D reaction hit testing, count toggles, picker/Escape, clipboard failure, no chat/media requests, email login');
} finally {
  browser('close');
}
