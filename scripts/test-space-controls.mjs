// Browser regression using explicitly mocked API responses. Never writes real data.
// Run with the dev server: SPACES_TEST_WEB_URL=http://localhost:3000/spaces node scripts/test-space-controls.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const url = process.env.SPACES_TEST_WEB_URL ?? 'http://localhost:3000/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname), 'Use a loopback preview');
const directory = mkdtempSync(join(tmpdir(), 'caper-space-controls-'));
const init = join(directory, 'fixture.js');
function fixture() {
  if (location.protocol === 'about:') return;
  const saved = new URL(location.href).searchParams.get('width') ?? '240';
  localStorage.setItem('caper:channel-sidebar-width', saved);
  const account = { id: 'owner1234567', username: 'fixture_owner', displayName: 'Fixture owner' };
  const space = { id: 'space1234567', name: 'Disposable UI fixture', ownerId: account.id };
  const channel = { id: 'channel12345', spaceId: space.id, name: 'fixture-channel', private: true };
  const control = window.spaceControlFixture = { deletes: [], fail: false, release: null, frames: [] };
  let deletedChannel = false, deletedSpace = false;
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
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
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels: deletedChannel ? [] : [channel], members: [] });
    if (path.endsWith('/messages')) return Response.json({ space, channel, messages: [], cursor: '0', hasMore: false });
    return Response.json({ error: 'Disabled in UI fixture' }, { status: 503 });
  };
  const start = performance.now();
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
    if (performance.now() - start < 5000) requestAnimationFrame(sample);
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
const modal = '.delete-confirmation';
const opens = () => evaluate('document.querySelectorAll(".space-dialog[open]").length');
function openOverview() {
  browser('focus', '[aria-label="Manage fixture-channel"]');
  browser('press', 'Enter');
  browser('click', '.danger-outline');
}
try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  for (const [viewport, saved, expected] of [[1280, '240', 240], [1280, '440', 440], [800, '440', 362], [1280, 'invalid', 280]]) {
    browser('set', 'viewport', String(viewport), '900', '2');
    browser('open', `${url}?space=space1234567&channel=channel12345&width=${saved}`);
    wait('!!document.querySelector(".channel-navigation")');
    const frames = evaluate('spaceControlFixture.frames');
    assert.ok(frames.some(f => f.loading) && frames.some(f => !f.loading));
    for (const f of frames) {
      assert.equal(f.geometry[3][2], expected, `Wrong sidebar width during ${f.loading ? 'loading' : 'loaded'} state`);
      assert.deepEqual(f.geometry, frames.at(-1).geometry, 'Loading shell moved');
    }
  }
  openOverview();
  assert.equal(opens(), 2);
  assert.equal(evaluate('document.activeElement.textContent'), 'Cancel');
  browser('press', 'Escape');
  assert.equal(opens(), 1, 'Escape must close only the confirmation');
  assert.equal(evaluate('document.activeElement.className'), 'danger-outline');
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
  browser('click', '.danger-outline');
  assert.match(evaluate('document.querySelector(".delete-confirmation").textContent'), /and all its channels for everyone\? This cannot be undone/);
  browser('click', `${modal} .danger`);
  wait('spaceControlFixture.deletes.length === 3');
  evaluate('spaceControlFixture.release()');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.equal(evaluate('spaceControlFixture.deletes.at(-1)'), '/api/spaces/space1234567');
  console.log('PASS: stable loading geometry, safe confirmation focus/dismissal/double-click, pending/failure/retry, channel and space deletion (mock API).');
} finally {
  try { browser('close'); } finally { rmSync(directory, { recursive: true, force: true }); }
}
