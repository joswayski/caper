// Real Call UI with a mocked spectator roster, not live SFU/device validation.
// Run against Vite: node scripts/test-country-flags.mjs http://localhost:5174
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const origin = new URL(process.argv[2] ?? 'http://localhost:5174');
assert.ok(['localhost', '127.0.0.1'].includes(origin.hostname), 'Local development only');
const session = `flags-${process.pid}`;
const artifacts = process.env.FLAG_TEST_ARTIFACTS && resolve(process.env.FLAG_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', session, ...args, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = code => browser('eval', `(async () => { ${code} })()`).result;

try {
  browser('open', origin.href);
  browser('set', 'viewport', '1280', '900', '2');
  evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {EventConnection} = await import('/src/media/event-connection.ts');
    const {PublicCallClient} = await import('/src/media/client.ts');
    const {default: Call} = await import('/src/pages/Call.tsx');
    PublicCallClient.prototype.prepareMicrophone = () => {};
    EventConnection.prototype.openPresence = async function () {
      this.snapshot({revision: 1, participants: [
        {id: 'account', name: 'Signed-in participant', muted: false, deafened: false},
        {id: 'guest', name: 'Located guest', countryCode: 'US', muted: false, deafened: false},
        {id: 'unknown', name: 'Unknown-location guest', muted: false, deafened: false},
      ]});
    };
    const originalFetch = window.fetch.bind(window);
    window.fetch = (input, options) => new URL(typeof input === 'string' ? input : input.url, location.href).pathname.endsWith('/media/status')
      ? Promise.resolve(Response.json({enabled: true})) : originalFetch(input, options);
    for (const element of document.body.children) element.hidden = true;
    const label = document.createElement('p');
    label.textContent = 'UI regression fixture · mocked API roster, no live participants';
    document.body.append(label);
    const mount = document.createElement('div'); mount.id = 'flag-fixture'; document.body.append(mount);
    const root = createRoot(mount);
    root.render(React.createElement(Call, {
      engaged: false,
      initialAccount: {id: 'viewer', username: 'viewer', displayName: 'Signed-in viewer'},
      initialHistory: {space: {id: 'fixture', name: 'Fixture'}, channel: {id: 'general', name: 'general'}, messages: [], cursor: '0', hasMore: false},
    }));
    window.cleanupFlags = () => root.unmount();
  `);
  browser('wait', '--fn', 'document.querySelectorAll("#flag-fixture .participant").length === 3');
  assert.deepEqual(evaluate(`return [...document.querySelectorAll('#flag-fixture .participant')].map(row => ({name: row.querySelector('strong').textContent, flag: !!row.querySelector('.participant-country')}));`), [
    { name: 'Signed-in participant', flag: false },
    { name: 'Located guest', flag: false },
    { name: 'Unknown-location guest', flag: false },
  ]);
  if (artifacts) browser('screenshot', '--full', `${artifacts}/roster-desktop.png`);
  browser('set', 'viewport', '390', '844', '2');
  evaluate('await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));');
  assert.equal(evaluate('return document.querySelectorAll("#flag-fixture .participant-country").length;'), 0);
  if (artifacts) browser('screenshot', '--full', `${artifacts}/roster-narrow.png`);
  evaluate('cleanupFlags();');
  console.log('PASS: desktop/narrow roster has no flags, including legacy countryCode payloads (mocked API projection)');
} finally {
  browser('close');
}
