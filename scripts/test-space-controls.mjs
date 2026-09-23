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
  const control = window.spaceControlFixture = { deletes: [], updates: [], fail: false, release: null, frames: [] };
  let deletedChannel = false, deletedSpace = false;
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
    if (options.method === 'PATCH') {
      const body = JSON.parse(options.body);
      control.updates.push({ path, body });
      const target = path.includes('/channels/') ? channel : space;
      Object.assign(target, body);
      return Response.json(target);
    }
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
    if (!control.frames.some(frame => !frame.loading)) requestAnimationFrame(sample);
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
    wait('spaceControlFixture.frames.some(frame => !frame.loading)');
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
  assert.equal(evaluate('getComputedStyle(document.activeElement).outlineStyle'), 'solid');
  browser('press', 'Enter');
  assert.equal(opens(), 1, 'Immediate Enter must cancel, not delete');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 0);
  browser('click', '.danger-outline');
  browser('press', 'Escape');
  assert.equal(opens(), 1, 'Escape must close only the confirmation');
  assert.equal(evaluate('document.activeElement.className'), 'danger-outline');
  browser('fill', '.space-field input', '   Fresh Plans   ');
  browser('press', 'Enter');
  wait('!document.querySelector(".channel-save-bar")');
  assert.equal(evaluate('document.querySelector(".space-field input").value'), 'fresh-plans');
  assert.deepEqual(evaluate('spaceControlFixture.updates.at(-1).body'), { name: 'fresh-plans', private: true });
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
  browser('fill', '.space-field input', '   Renamed studio   ');
  browser('press', 'Enter');
  wait('document.querySelector(".space-field input").value === "Renamed studio"');
  assert.deepEqual(evaluate('spaceControlFixture.updates.at(-1).body'), { name: 'Renamed studio' });
  browser('fill', '.space-field input', '   Renamed studio   ');
  browser('press', 'Tab');
  assert.equal(evaluate('document.querySelector(".space-field input").value'), 'Renamed studio', 'Whitespace-only edits normalize on blur even without saving');
  browser('click', '.danger-outline');
  assert.match(evaluate('document.querySelector(".delete-confirmation").textContent'), /All its channels and their messages will disappear from the space\. This cannot be undone/);
  browser('press', 'Enter');
  assert.equal(opens(), 1, 'Immediate Enter also cancels space deletion');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 2);
  browser('click', '.danger-outline');
  browser('click', `${modal} .danger`);
  wait('spaceControlFixture.deletes.length === 3');
  evaluate('spaceControlFixture.release()');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.equal(evaluate('spaceControlFixture.deletes.at(-1)'), '/api/spaces/space1234567');
  console.log('PASS: stable loading geometry, safe confirmation focus/Enter/dismissal/double-click, trimmed name updates, pending/failure/retry, channel and space deletion (mock API).');
} finally {
  try { browser('close'); } finally { rmSync(directory, { recursive: true, force: true }); }
}
