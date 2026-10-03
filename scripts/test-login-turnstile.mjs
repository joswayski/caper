// Local browser integration tests. Turnstile callbacks and account responses are
// explicit fixtures; no production authentication, SES, or bot detection runs.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const origin = new URL(process.argv[2] ?? 'http://localhost:5174');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname), 'Local development only');
const artifacts = process.argv[3] && resolve(process.argv[3]);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const session = `login-${process.pid}`;
const fixture = mkdtempSync(join(tmpdir(), 'caper-turnstile-'));
const initScript = join(fixture, 'fixture.js');
writeFileSync(initScript, `
  window.loginFixture = { widgets: [], requests: [], status: 202, config: new URLSearchParams(location.search).get('fixture-config') ?? 'enabled' };
  window.turnstile = {
    render(container, options) {
      const id = String(window.loginFixture.widgets.length);
      const widget = { id, options, removed: false };
      window.loginFixture.widgets.push(widget);
      return id;
    },
    remove(id) { window.loginFixture.widgets[Number(id)].removed = true; },
  };
  const fetchOriginal = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const path = String(input);
    if (path === '/api/account/me') return Response.json({ error: 'unauthorized' }, { status: 401 });
    if (path === '/api/auth/config') {
      if (window.loginFixture.config === 'failed') return Response.json({}, { status: 503 });
      return Response.json({ turnstileSiteKey: window.loginFixture.config === 'disabled' ? null : 'fixture-site-key' });
    }
    if (path === '/api/auth/email/request') {
      window.loginFixture.requests.push(JSON.parse(init.body));
      const status = window.loginFixture.status;
      return Response.json(status === 202 ? { challengeId: 'fixture-challenge-' + window.loginFixture.requests.length } : { error: 'fixture failure' }, { status });
    }
    if (path === '/api/auth/email/verify') return Response.json({ error: 'expired', attemptsRemaining: 0 }, { status: 401 });
    return fetchOriginal(input, init);
  };
`);
function browser(...args) {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', session, '--init-script', initScript, ...args, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = code => browser('eval', `(async () => { ${code} })()`).result;
const wait = code => browser('wait', '--fn', `Boolean(${code})`);
const current = 'window.loginFixture.widgets.at(-1)';
const solve = token => evaluate(`${current}.options.callback(${JSON.stringify(token)});`);
const sendButton = () => evaluate('return [...document.querySelectorAll("button")].find(button => /Email me/.test(button.textContent)).disabled');
const capture = name => {
  if (!artifacts) return;
  evaluate('await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));');
  browser('screenshot', join(artifacts, name));
};

try {
  browser('open', origin.href);
  browser('network', 'route', '**/turnstile/v0/api.js*', '--body', '/* Turnstile lifecycle is mocked by the init script. */', '--content-type', 'application/javascript');
  browser('set', 'viewport', '1280', '800', '2');
  browser('open', new URL('/login', origin).href);
  wait(`${current}`);
  browser('fill', '#email', 'person@caper.chat');
  assert.equal(sendButton(), true);
  evaluate('document.querySelector("form").requestSubmit();');
  assert.equal(evaluate('return window.loginFixture.requests.length'), 0, 'Missing verification cannot send');
  assert.deepEqual(evaluate(`return [${current}.options.action, ${current}.options.appearance]`), ['login_email', 'interaction-only']);
  solve('expired-fixture-token');
  wait('!document.querySelector("button[type=submit]").disabled');
  evaluate(`${current}.options['expired-callback']();`);
  wait('document.querySelector("button[type=submit]").disabled');
  assert.equal(sendButton(), true, 'An expired token immediately disables sending');
  solve('first-fixture-token');
  evaluate('window.loginFixture.status = 403; window.oldWidget = window.loginFixture.widgets.at(-1);');
  browser('click', 'button[type=submit]');
  wait('document.querySelector("[role=alert]")?.textContent.includes("Complete the verification")');
  wait('window.oldWidget.removed');
  assert.deepEqual(evaluate('return window.loginFixture.requests'), [{ email: 'person@caper.chat', turnstileToken: 'first-fixture-token' }]);
  evaluate('window.oldWidget.options.callback("stale-fixture-token");');
  assert.equal(sendButton(), true, 'Disposed widgets cannot restore a spent token');
  evaluate(`${current}.options['error-callback']();`);
  wait('document.body.textContent.includes("Verification couldn’t load")');
  capture('login-verification-error-mock.png');
  browser('find', 'role', 'button', 'click', '--name', 'Try again', '--exact');
  wait('!document.body.textContent.includes("Verification couldn’t load")');
  solve('second-fixture-token');
  evaluate('window.loginFixture.status = 202;');
  browser('click', 'button[type=submit]');
  wait('document.querySelector("#code")');
  assert.equal(evaluate(`return ${current}.removed`), true, 'Code verification does not retain the widget');
  browser('fill', '#code', 'AAAAAA');
  browser('click', 'button[type=submit]');
  wait('document.querySelector("#code").disabled');
  wait(`!${current}.removed`);
  assert.equal(sendButton(), true, 'Resending requires a new verification');
  solve('resend-fixture-token');
  evaluate('window.loginFixture.status = 503; window.oldWidget = window.loginFixture.widgets.at(-1);');
  browser('find', 'role', 'button', 'click', '--name', 'Email me a new code');
  wait('document.body.textContent.includes("Sign-in is temporarily unavailable")');
  wait('window.oldWidget.removed');
  assert.equal(evaluate('return document.querySelector("#code").disabled'), true, 'Failed resend preserves the exhausted-code state');
  assert.equal(sendButton(), true, 'Failed resend cannot reuse its token');
  assert.equal(evaluate('return window.loginFixture.requests.at(-1).turnstileToken'), 'resend-fixture-token');
  browser('set', 'viewport', '390', '844', '2');
  capture('login-resend-error-narrow-mock.png');
  browser('find', 'role', 'button', 'click', '--name', 'Use a different email', '--exact');
  wait('document.querySelector("#email")');
  for (const [width, size] of [[1280, 'flexible'], [390, 'flexible'], [320, 'compact']]) {
    browser('set', 'viewport', String(width), '844', '2');
    wait(`${current}.options.size === '${size}'`);
    assert.ok(evaluate('return document.documentElement.scrollWidth <= innerWidth'), `No overflow at ${width}px`);
  }
  console.log('PASS: missing/expired/spent tokens cannot send; failed verification and provider errors recover with fresh tokens');
  console.log('PASS: exhausted-code resend retains its state after failure; widgets clean up; desktop/390px/320px sizing works');

  browser('open', new URL('/login?fixture-config=disabled', origin).href);
  wait('document.querySelector("#email") && !document.querySelector("button[type=submit]").disabled');
  assert.equal(evaluate(`return document.querySelector('[aria-label="Browser verification"]')`), null);
  browser('fill', '#email', 'person@caper.chat');
  browser('click', 'button[type=submit]');
  wait('document.querySelector("#code")');
  assert.deepEqual(evaluate('return window.loginFixture.requests'), [{ email: 'person@caper.chat' }]);
  console.log('PASS: unconfigured Turnstile keeps the ordinary login flow');

  browser('open', new URL('/login?fixture-config=failed', origin).href);
  wait('document.body.textContent.includes("Sign-in is temporarily unavailable")');
  assert.equal(sendButton(), true, 'Configuration failure cannot skip verification');
  assert.deepEqual(evaluate('return window.loginFixture.requests'), []);
  console.log('PASS: unavailable configuration fails closed');
} catch (error) {
  console.error(browser('snapshot'));
  console.error(browser('errors'));
  throw error;
} finally {
  try { browser('close'); } finally { rmSync(fixture, { recursive: true, force: true }); }
}
