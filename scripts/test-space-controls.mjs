// Browser regression using explicitly mocked API responses. Never writes real data.
// Run with the dev server: SPACES_TEST_WEB_URL=http://localhost:3000/spaces node scripts/test-space-controls.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const url = process.env.SPACES_TEST_WEB_URL ?? 'http://localhost:3000/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname), 'Use a loopback preview');
const artifacts = process.env.SPACES_TEST_ARTIFACTS && resolve(process.env.SPACES_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(join(tmpdir(), 'caper-space-controls-'));
const init = join(directory, 'fixture.js');
function fixture() {
  if (location.protocol === 'about:') return;
  const saved = new URL(location.href).searchParams.get('width') ?? '240';
  const publicDemo = new URL(location.href).searchParams.has('public');
  const guest = new URL(location.href).searchParams.has('guest');
  localStorage.setItem('caper:channel-sidebar-width', saved);
  const account = { id: 'owner1234567', username: 'fixture_owner', displayName: 'Fixture owner', avatarId: 0, debugEnabled: new URL(location.href).searchParams.has('debug') };
  const space = { id: 'space1234567', name: 'Disposable UI fixture', ownerId: account.id };
  const channel = { id: 'channel12345', spaceId: space.id, name: 'fixture-channel', private: !new URL(location.href).searchParams.has('publicChannel') };
  window.homeFixture = { account, history: { space: { id: 'public123456', name: 'Public demo' }, channel: { id: 'general12345', name: 'general' }, messages: [], cursor: '0', hasMore: false } };
  const members = [{ ...account, owner: true }, ...Array.from({ length: 29 }, (_, index) => ({
    id: `member${String(index).padStart(6, '0')}`, username: `member_${index}`, displayName: `Fixture member ${index + 1}`, avatarId: index % 2 ? 31 : 799, owner: false,
  }))];
  const control = window.spaceControlFixture = { deletes: [], updates: [], memberAdds: [], fail: false, release: null, frames: [], subscriptions: {} };
  let deletedChannel = false, deletedSpace = false;
  const originalFetch = window.fetch.bind(window);
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    subscriptions = new Map();
    constructor(socketUrl, protocols) {
      super();
      if (!String(socketUrl).includes('/api/chat/events')) return new NativeSocket(socketUrl, protocols);
      control.disconnect = () => { control.disconnected = true; this.dispatchEvent(new Event('close')); };
      control.setPresence = (status) => {
        for (const request of this.subscriptions.values()) {
          if (request.kind === 'presence') this.frame({ type: 'event', id: request.id, event: { type: 'snapshot', members: request.userIds.map(userId => ({ userId, status })) } });
        }
      };
      if (!control.disconnected) queueMicrotask(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (request.type === 'unsubscribe') { this.subscriptions.delete(request.id); delete control.subscriptions[request.id]; }
      if (request.type === 'subscribe') {
        this.subscriptions.set(request.id, request);
        control.subscriptions[request.id] = request;
        const event = request.kind === 'chat'
          ? { type: 'ready', cursor: request.after ?? '0' }
          : request.kind === 'presence'
            ? { type: 'snapshot', members: request.userIds.map((userId, index) => ({ userId, status: ['online', 'idle', 'offline'][index % 3] })) }
            : { type: 'snapshot', participants: [], revision: 1 };
        if (request.kind !== 'presence' || !control.holdPresence) this.frame({ type: 'event', id: request.id, event });
        this.frame({ type: 'subscribed', id: request.id });
      }
    }
    frame(value) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(value) })); }
    close() {}
  };
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
    if (path.endsWith('/members') && options.method === 'POST') {
      const body = JSON.parse(options.body);
      control.memberAdds.push({ path, body });
      await new Promise(resolve => { control.memberRelease = resolve; });
      control.memberRelease = null;
      const member = { id: 'addedmember12', username: body.username, displayName: 'Added fixture member', avatarId: 32, owner: false };
      members.push(member);
      return Response.json(member);
    }
    if (path === '/api/account/profile' && options.method === 'POST') {
      await new Promise(resolve => { control.profileRelease = resolve; });
      control.profileRelease = null;
      return Response.json({ error: 'Fixture username conflict' }, { status: 409 });
    }
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
      if (guest) return Response.json({error:'unauthorized'}, {status:401});
      return Response.json(account);
    }
    if (path === '/api/chat/session') return Response.json({token:'fixture-token',author:{id:account.id,name:guest?'Fixture guest':account.displayName,isGuest:guest}});
    if (publicDemo && path === '/api/chat/general') return Response.json({space,channel,messages:[],cursor:'0',hasMore:false});
    if (path === '/api/spaces') return Response.json({ spaces: deletedSpace ? [] : [space], limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 } });
    if (path.endsWith('/members')) return Response.json({ members: [{ ...account, owner: true }] });
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels: deletedChannel ? [] : [channel], members });
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
const screenshot = name => { if (artifacts) browser('screenshot', ...(name.startsWith('homepage-') ? [] : ['--full']), `${artifacts}/${name}.png`); };
const modal = '.delete-confirmation';
const opens = () => evaluate('document.querySelectorAll(".space-dialog[open]").length');
function openOverview() {
  browser('focus', '[aria-label="Manage fixture-channel"]');
  browser('press', 'Enter');
  browser('find', 'role', 'button', 'click', '--name', 'Channel settings', '--exact');
  browser('click', '.danger-outline');
  wait('!!document.querySelector(".delete-confirmation")');
}
function geometry(selector) {
  return evaluate(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()`);
}
function dismissMemberBackdrop() {
  const [x, y, width, height] = geometry('.member-list-backdrop');
  assert.ok(width > 64 && height > 40, 'Member backdrop must cover the conversation body');
  browser('mouse', 'move', String(x + 8), String(y + 20));
  browser('mouse', 'down', 'left');
  browser('mouse', 'up', 'left');
}
function dismissBackdrop() {
  browser('mouse', 'move', '1', '1');
  browser('mouse', 'down', 'left');
  browser('mouse', 'up', 'left');
}
function stableShell(selector) {
  const before = geometry(selector);
  evaluate(`(() => { const d = document.querySelector(${JSON.stringify(selector)}); const p = document.createElement('p'); p.dataset.layoutProbe = ''; p.textContent = 'Test-only asynchronous content '.repeat(200); d.append(p); })()`);
  assert.deepEqual(geometry(selector), before, `${selector}: overflowing async content must not resize or recenter`);
  evaluate('document.querySelector("[data-layout-probe]").remove()');
  assert.deepEqual(geometry(selector), before);
}
function testModalGeometry() {
  for (const [width, height] of [[1280, 900], [390, 844], [390, 500]]) {
    browser('set', 'viewport', String(width), String(height), '2');
    browser('open', `${url}?space=space1234567&channel=channel12345&publicChannel&debug`);
    wait('!!document.querySelector(".channel-navigation")');
    evaluate('document.fonts.ready');
    if (width < 760) browser('click', '.navigation-toggle');
    if (width < 760) {
      const layout = evaluate(`(() => {
        const rail = document.querySelector('.space-rail'), sidebar = document.querySelector('.sidebar-channels'), account = document.querySelector('.call-account');
        return { railBorder: getComputedStyle(rail).borderRightWidth, sidebar: sidebar.getBoundingClientRect().toJSON(), account: account.getBoundingClientRect().toJSON(), room: document.querySelector('.call-room').getBoundingClientRect().toJSON() };
      })()`);
      assert.equal(layout.railBorder, '0px', 'The mobile rail must not draw a full-height divider');
      assert.ok(layout.sidebar.y > layout.room.y && layout.sidebar.right < width, 'Channel surface must be inset at the top and right');
      assert.ok(layout.account.x < 60 && layout.account.right <= width - 8, 'Account bar must span beneath both navigation columns');
      assert.ok(layout.account.y >= layout.sidebar.bottom && layout.account.bottom < layout.room.bottom, 'Account controls must align below the channel surface and clear the bottom edge');
      screenshot(`navigation-inset-${width}-${height}`);
    }
    browser('focus', '[aria-label="Manage fixture-channel"]');
    browser('press', 'Enter');
    browser('find', 'role', 'button', 'click', '--name', 'Channel settings', '--exact');
    const selectors = ['.space-dialog[open]', '.space-field input', '.channel-privacy input'];
    const before = selectors.map(geometry);
    assert.ok(evaluate('document.querySelector(".channel-save-bar").inert'));
    screenshot(`modal-channel-clean-${width}-${height}`);
    browser('check', '.channel-privacy input');
    assert.deepEqual(selectors.map(geometry), before, 'Privacy toggle moved dialog or controls');
    assert.equal(evaluate('document.querySelector(".channel-save-bar").inert'), false);
    screenshot(`modal-channel-dirty-${width}-${height}`);
    browser('click', '.channel-save-bar .secondary');
    assert.deepEqual(selectors.map(geometry), before, 'Reset moved dialog or controls');
    browser('check', '.channel-privacy input');
    browser('click', '.channel-save-bar .primary');
    wait('document.querySelector(".channel-save-bar").inert && !!document.querySelector(".member-manager")');
    assert.deepEqual(selectors.map(geometry), before, 'Saving privacy and loading members moved dialog or fields');
    screenshot(`modal-channel-members-${width}-${height}`);
    browser('uncheck', '.channel-privacy input');
    assert.deepEqual(selectors.map(geometry), before, 'Making a private channel public moved dialog or fields');
    browser('click', '.channel-save-bar .secondary');
    stableShell('.space-dialog[open]');
    evaluate('document.querySelector(".space-dialog[open]").scrollTop = document.querySelector(".space-dialog[open]").scrollHeight');
    assert.ok(evaluate(`(() => { const button = document.querySelector('.danger-outline'); const r = button.getBoundingClientRect(); return button.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })()`), 'Scrolled delete action must be pointer-reachable');
    screenshot(`modal-channel-scrolled-${width}-${height}`);
    browser('click', '.danger-outline');
    wait('!!document.querySelector(".delete-confirmation")');
    screenshot(`modal-delete-${width}-${height}`);
    stableShell('.space-dialog:has(.delete-confirmation)');
    dismissBackdrop();
    assert.equal(opens(), 1, 'Confirmation backdrop must leave settings open');
    evaluate('document.querySelector(".space-dialog[open]").scrollTop = 0');
    browser('click', '.space-field input');
    assert.equal(opens(), 1, 'Inside click must leave channel settings open');
    dismissBackdrop();
    assert.equal(opens(), 0, 'Channel settings backdrop must dismiss');
    for (const label of ['Create space', 'Create channel']) {
      browser('click', `[aria-label="${label}"]`);
      screenshot(`modal-${label.toLowerCase().replace(' ', '-')}-${width}-${height}`);
      stableShell('.space-dialog[open]');
      browser('press', 'Escape');
    }
    browser('click', '.space-menu summary');
    browser('click', '.space-actions button');
    screenshot(`modal-manage-space-${width}-${height}`);
    stableShell('.space-dialog[open]');
    browser('fill', '#member-username', 'fixture_new');
    assert.equal(evaluate('document.querySelector("#member-username").getAttribute("autocapitalize")'), 'none');
    if (width < 760) assert.equal(evaluate('getComputedStyle(document.querySelector("#member-username")).fontSize'), '16px', 'iOS forms must not trigger focus zoom');
    browser('press', 'Enter');
    wait('!!spaceControlFixture.memberRelease');
    assert.deepEqual(evaluate('spaceControlFixture.memberAdds'), [{ path: '/api/spaces/space1234567/members', body: { username: 'fixture_new' } }]);
    assert.ok(evaluate('document.querySelector("#member-username").disabled'), 'Pending add must not accept edits that will be cleared');
    evaluate('document.querySelector(".member-add").requestSubmit()');
    assert.equal(evaluate('spaceControlFixture.memberAdds.length'), 1, 'Pending keyboard resubmission must not duplicate membership writes');
    evaluate('spaceControlFixture.memberRelease()');
    wait('document.querySelector("#member-username").value === "" && document.querySelector(".member-manager").textContent.includes("@fixture_new")');
    screenshot(`modal-member-added-${width}-${height}`);
    evaluate('document.querySelector(".space-dialog[open]").scrollTop = 0');
    browser('click', '.space-field input');
    assert.equal(opens(), 1, 'Inside click must leave space settings open');
    dismissBackdrop();
    assert.equal(opens(), 0, 'Space settings backdrop must dismiss');
    browser('click', '.account-profile');
    wait('!!document.querySelector(".profile-dialog[open] form")');
    // Normalize scroll before measuring: a short viewport legitimately scrolls
    // to an off-screen submit button when it is clicked.
    evaluate('document.querySelector(".profile-dialog button[type=submit]").scrollIntoView({block:"nearest"})');
    const profileSelectors = ['.profile-dialog[open]', '#username', '#display-name', '.profile-dialog button[type=submit]'];
    const profileBefore = profileSelectors.map(geometry);
    browser('click', '.profile-dialog button[type=submit]');
    wait('!!spaceControlFixture.profileRelease');
    assert.deepEqual(profileSelectors.map(geometry), profileBefore, 'Saving moved profile');
    evaluate('spaceControlFixture.profileRelease()');
    wait('!!document.querySelector(".profile-dialog [role=alert]")');
    assert.deepEqual(profileSelectors.map(geometry), profileBefore, 'Error moved profile');
    screenshot(`modal-profile-error-${width}-${height}`);
    browser('click', '.profile-dialog button[type=submit]');
    wait('!!spaceControlFixture.profileRelease');
    assert.deepEqual(profileSelectors.map(geometry), profileBefore, 'Error-to-saving moved profile');
    screenshot(`modal-profile-saving-${width}-${height}`);
    evaluate('spaceControlFixture.profileRelease()');
    wait('!!document.querySelector(".profile-dialog [role=alert]")');
    stableShell('.profile-dialog[open]');
    browser('press', 'Escape');
    browser('click', '[aria-label="User Settings"]');
    browser('find', 'role', 'button', 'click', '--name', 'Audio diagnostics', '--exact');
    wait('!!document.querySelector(".audio-dialog[open]")');
    screenshot(`modal-audio-${width}-${height}`);
    stableShell('.audio-dialog[open]');
    browser('press', 'Escape');
    if (width < 760) {
      browser('click', '.channel-select[aria-current="page"]');
      browser('click', '.member-list-toggle');
      wait('!!document.querySelector(".member-list-close")');
      screenshot(`members-overlay-${width}-${height}`);
      const closeBounds = geometry('.member-list-close');
      assert.ok(evaluate('document.querySelector(".space-member-presence").scrollHeight > document.querySelector(".space-member-presence").clientHeight'));
      evaluate('document.querySelector(".space-member-presence").scrollTop = document.querySelector(".space-member-presence").scrollHeight');
      assert.deepEqual(geometry('.member-list-close'), closeBounds, 'Close must remain visible while the member list scrolls');
      screenshot(`members-scrolled-${width}-${height}`);
      browser('click', '.member-list-close');
      assert.equal(evaluate('!!document.querySelector(".space-member-presence")'), false);
      browser('click', '.member-list-toggle');
      dismissMemberBackdrop();
      assert.equal(evaluate('!!document.querySelector(".space-member-presence")'), false, 'Outside tap must close the member panel');
      assert.equal(evaluate('!!document.querySelector(".member-list-backdrop")'), false);
    }
    assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Modal overflowed viewport');
  }
  console.log('PASS: desktop/narrow/short modal geometry, editable exact-username add/Enter/pending guard, mobile inset navigation/account alignment, and member Close/outside dismissal (mock API/gateway).');
}
try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  testModalGeometry();
  if (!process.env.MODALS_ONLY) {
  if (!process.env.HOMEPAGE_ONLY) {
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
  wait('document.querySelectorAll(".space-member-presence li").length === 25 && !document.querySelector(".member-presence-connecting")');
  assert.ok(evaluate('(() => { const a = document.querySelector(".channel-navigation > header").getBoundingClientRect(), b = document.querySelector(".chat-heading").getBoundingClientRect(); return a.top === b.top && a.bottom === b.bottom; })()'), 'Space and channel headers must align');
  assert.ok(evaluate('document.querySelector(".space-member-presence").getBoundingClientRect().left >= document.querySelector(".stage").getBoundingClientRect().right'), 'Desktop members must be on the right');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).find(s => s.kind === "presence" && s.userIds.length > 1).userIds.length'), 25);
  assert.equal(evaluate('document.querySelector(".account-avatar .presence-dot").getAttribute("aria-label")'), 'Online');
  assert.equal(evaluate('document.querySelectorAll(".space-member-presence small").length'), 0, 'Statuses belong on the dots, not text rows');
  assert.equal(evaluate('getComputedStyle(document.querySelector(".member-presence-heading")).borderBottomWidth'), '0px');
  assert.equal(evaluate('document.querySelector(".channel-section-toggle .section-count").textContent'), '1');
  assert.equal(evaluate('document.querySelector(".member-presence-heading .section-count").textContent'), '30', 'Member count must include every page');
  screenshot('gateway-merged-members-desktop');
  for (const status of ['idle', 'offline', 'online']) {
    evaluate(`spaceControlFixture.setPresence('${status}')`);
    wait(`document.querySelector('.account-avatar .presence-dot').dataset.status === '${status}'`);
    assert.equal(evaluate('document.querySelector(".space-member-presence .presence-dot").dataset.status'), status);
    screenshot(`presence-${status}`);
  }
  browser('click', '.member-presence-pages button:last-child');
  wait('document.querySelectorAll(".space-member-presence li").length === 5 && !document.querySelector(".member-presence-connecting")');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).filter(s => s.kind === "presence").length'), 2);
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).find(s => s.kind === "presence" && s.userIds.length > 1).userIds.length'), 5);
  const expandedChatWidth = evaluate('document.querySelector(".stage").getBoundingClientRect().width');
  browser('click', '.member-list-toggle');
  wait('!Object.values(spaceControlFixture.subscriptions).some(s => s.kind === "presence" && s.userIds.length > 1)');
  assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).filter(s => s.kind === "presence").length'), 1, 'Own presence must remain subscribed with members hidden');
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null, 'Hidden members must not retain a sidebar');
  assert.equal(evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded")'), 'false');
  assert.ok(evaluate('document.querySelector(".stage").getBoundingClientRect().width') > expandedChatWidth, 'Chat must reclaim the member column');
  browser('click', '.channel-section-toggle');
  assert.ok(evaluate('document.querySelector("#space-channel-list").hidden'));
  assert.equal(evaluate('document.querySelector(".channel-section-toggle .section-count").textContent'), '1');
  screenshot('members-collapsed');
  browser('click', '.channel-section-toggle');
  browser('click', '.member-list-toggle');
  wait('document.querySelectorAll(".space-member-presence li").length === 25');
  assert.equal(evaluate('document.querySelector(".member-presence-heading .section-count").textContent'), '30');
  assert.ok(evaluate('[...document.querySelectorAll(".space-member-presence .presence-dot")].every(node => ["online", "idle", "offline"].includes(node.dataset.status))'), 'Reopening must restore live subscriptions');
  assert.equal(evaluate('document.querySelector(".space-member-presence").textContent.includes("Updating")'), false);
  browser('set', 'viewport', '390', '844', '2');
  screenshot('members-narrow-open');
  browser('click', '.member-list-close');
  assert.equal(evaluate('!!document.querySelector(".space-member-presence")'), false);
  browser('click', '.member-list-toggle');
  dismissMemberBackdrop();
  assert.equal(evaluate('!!document.querySelector(".space-member-presence")'), false, 'Outside tap must close the narrow member panel');
  assert.equal(evaluate('!!document.querySelector(".member-list-backdrop")'), false);
  screenshot('members-narrow-hidden');
  assert.ok(evaluate('Math.abs(document.querySelector(".chat-messages").getBoundingClientRect().bottom - document.querySelector(".chat-typing").getBoundingClientRect().top) < 2'), 'Mobile messages must fill the available grid row without a fixed-height blank gap');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Narrow member navigation must not overflow');
  screenshot('gateway-merged-members-narrow');
  browser('click', '.channel-select[aria-current="page"]');
  wait('!document.querySelector(".spaces-room.navigation-open")');
  browser('set', 'viewport', '1280', '900', '2');
  for (const label of ['Create space', 'Create channel']) {
    browser('click', `[aria-label="${label}"]`);
    wait('!!document.querySelector(".space-dialog[open]")');
    browser('click', '.space-field input');
    assert.equal(opens(), 1, 'Clicking inside must not dismiss');
    screenshot(label === 'Create space' ? 'create-space-dialog' : 'create-channel-dialog');
    browser('mouse', 'move', '10', '10');
    browser('mouse', 'down', 'left');
    browser('mouse', 'up', 'left');
    assert.equal(opens(), 0, `${label} must dismiss on backdrop click`);
  }
  openOverview();
  assert.equal(opens(), 2);
  assert.equal(evaluate('document.activeElement.textContent'), 'Cancel');
  assert.equal(evaluate('getComputedStyle(document.activeElement).outlineStyle'), 'solid');
  screenshot('gateway-merged-delete-confirmation');
  browser('press', 'Enter');
  assert.equal(opens(), 1, 'Immediate Enter must cancel, not delete');
  assert.equal(evaluate('spaceControlFixture.deletes.length'), 0);
  browser('click', '.danger-outline');
  browser('press', 'Escape');
  assert.equal(opens(), 1, 'Escape must close only the confirmation');
  assert.equal(evaluate('document.activeElement.className'), 'danger-outline');
  browser('fill', '.space-field input', '   Fresh Plans   ');
  browser('press', 'Enter');
  wait('document.querySelector(".channel-save-bar").inert');
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
  for (const enabled of [false, true]) {
    browser('set', 'viewport', '1280', '900', '2');
    browser('open', `${url}?space=space1234567&channel=channel12345${enabled ? '&debug' : ''}`);
    wait('!!document.querySelector(".account-avatar")');
    browser('click', '[aria-label="User Settings"]');
    assert.equal(evaluate('[...document.querySelectorAll("button")].some(b => b.textContent === "Audio diagnostics")'), enabled);
    if (enabled) {
      browser('find', 'role', 'button', 'click', '--name', 'Audio diagnostics', '--exact');
      wait('!!document.querySelector(".audio-debug")');
      assert.match(evaluate('document.querySelector(".audio-debug").textContent'), /No microphone capture started/);
      assert.equal(evaluate('JSON.parse(document.querySelector(".audio-debug pre").textContent).captureAttempt'), 'not-started');
    }
  }
  console.log('PASS: debug-enabled account sees diagnostics; other accounts do not; unused capture is explicit.');
  for (const guest of [false, true]) {
    browser('open', `${url}?public${guest ? '&guest' : ''}`);
    wait('document.querySelector(".account-avatar .presence-dot")?.dataset.status === "online"');
    assert.equal(evaluate('Object.values(spaceControlFixture.subscriptions).filter(s => s.kind === "presence").length'), 0, 'Demo self presence must not subscribe to account-space presence');
    assert.equal(evaluate('getComputedStyle(document.querySelector(".account-avatar .presence-dot")).backgroundColor'), 'rgb(99, 122, 67)');
    screenshot(guest ? 'guest-self-online' : 'demo-account-self-online');
    browser('click', '[aria-label="User Settings"]');
    assert.equal(evaluate('document.querySelector(".top-layer-tooltip:popover-open")'), null, 'Clicking settings dismisses its tooltip');
    browser('uncheck', '[role="switch"]');
    assert.equal(evaluate('localStorage.getItem("caper:system-sounds")'), 'off');
    screenshot(guest ? 'guest-system-sounds-off' : 'account-system-sounds-off');
    browser('open', `${url}?public${guest ? '&guest' : ''}`);
    wait('document.querySelector(".account-avatar .presence-dot")?.dataset.status === "online"');
    browser('click', '[aria-label="User Settings"]');
    assert.equal(evaluate('document.querySelector("[role=switch]").checked'), false, 'System sounds preference survives reload');
    browser('check', '[role="switch"]');
    screenshot(guest ? 'guest-system-sounds-on' : 'account-system-sounds-on');
    if (guest) {
      browser('set', 'viewport', '390', '844', '2');
      browser('click', '.navigation-toggle');
      browser('click', '[aria-label="User Settings"]');
      screenshot('guest-system-sounds-narrow');
      browser('set', 'viewport', '1280', '900', '2');
    }
    browser('click', '[aria-label="User Settings"]');
    evaluate('window.realNow = Date.now; Date.now = () => realNow() + 600001');
    wait('document.querySelector(".account-avatar .presence-dot").dataset.status === "idle"');
    screenshot(guest ? 'guest-self-idle' : 'demo-account-self-idle');
    evaluate('Date.now = realNow; window.dispatchEvent(new Event("pointerdown"))');
    wait('document.querySelector(".account-avatar .presence-dot").dataset.status === "online"');
    evaluate('spaceControlFixture.disconnect()');
    wait('document.querySelector(".account-avatar .presence-dot").dataset.status === "offline"');
  }
  console.log('PASS: public guest and account footer dots track chat online/idle/offline without an account presence subscription.');
  console.log('PASS: stable loading geometry, scoped member pagination/unsubscribe and narrow layout, safe confirmation focus/Enter/dismissal/double-click, trimmed name updates, pending/failure/retry, channel and space deletion (mock API/gateway).');
  }
  // Mount the real homepage with explicit loader fixtures; API/gateway mocks
  // remain the same as the standalone spaces checks above.
  for (const guest of [false, true]) {
    browser('open', `${url}?public&width=440`);
    wait('!!document.querySelector(".channel-navigation")');
    evaluate(`(async () => {
      const { default: React } = await import('/node_modules/.vite/deps/react.js');
      const { default: { createRoot } } = await import('/node_modules/.vite/deps/react-dom_client.js');
      const { default: Home } = await import('/src/pages/Home.tsx');
      for (const child of document.body.children) child.style.display = 'none';
      History.prototype.replaceState.call(history, {}, '', '/');
      if (${guest}) {
        const fetch = window.fetch;
        window.fetch = (input, options) => String(input) === '/api/account/me'
          ? Promise.resolve(Response.json({ error: 'unauthorized' }, { status: 401 }))
          : fetch(input, options);
      }
      const root = document.createElement('div'); document.body.append(root);
      createRoot(root).render(React.createElement(Home, { account: ${guest ? 'null' : 'homeFixture.account'}, history: homeFixture.history, initialNow: Date.now(), latestChanges: [] }));
    })()`);
    if (!guest) {
      wait('!!document.querySelector(".live-app .channel-navigation[data-demo] .channel-select")');
      assert.equal(evaluate('document.querySelectorAll(".live-app .channel-navigation h1").length'), 0, 'Public demo must not render a space heading');
      assert.equal(evaluate('document.querySelectorAll(".live-app .channel-section-toggle").length'), 0, 'The single-channel demo must not have a Channels dropdown');
      browser('set', 'viewport', '1280', '844', '2');
      evaluate('document.fonts.ready');
      screenshot('homepage-public-no-heading');
    }
    for (const width of [1280, 390]) {
      browser('set', 'viewport', String(width), '844', '2');
      browser('click', '.live-activator');
      wait('!!document.querySelector(".live-scene[data-settled], .live-stage[data-sheet]")');
      wait('[...document.querySelectorAll(".live-stage, .live-scene")].every(el => el.getAnimations().every(animation => animation.playState === "finished"))');
      assert.equal(evaluate('document.querySelectorAll(".live-titlebar, .live-lights").length'), 0);
      if (!guest) {
        if (width === 1280) {
          const alignment = evaluate(`(() => {
            const row = document.querySelector('.live-app .channel-select').getBoundingClientRect();
            const heading = document.querySelector('.live-app .chat-heading').getBoundingClientRect();
            return { row: row.toJSON(), heading: heading.toJSON(), delta: row.top + row.height / 2 - heading.top - heading.height / 2 };
          })()`);
          assert.ok(Math.abs(alignment.delta) < 2, `Public general row must align with the chat header: ${JSON.stringify(alignment)}`);
          assert.ok(evaluate('document.querySelector(".live-app .people-panel").getBoundingClientRect().width <= 260'), 'A wide saved sidebar must stay compact on the homepage');
          assert.ok(evaluate(`(() => {
            const line = document.querySelector('.live-app .channel-line').getBoundingClientRect();
            const panel = document.querySelector('.live-app .people-panel').getBoundingClientRect();
            return line.height <= 44 && line.top - panel.top >= 5 && line.top - panel.top <= 8;
          })()`), 'Public channel highlight must be compact and inset from the top');
          screenshot('homepage-public-aligned');
        }
        if (width === 390) browser('click', '.live-app .navigation-toggle');
        browser('click', '.live-app [aria-label="Disposable UI fixture"]');
        wait('!!document.querySelector(".live-app .channel-manage")');
        assert.equal(evaluate('document.querySelectorAll(".live-app .channel-section-toggle").length'), 1, 'Account spaces retain channel controls');
        assert.ok(evaluate('parseFloat(getComputedStyle(document.querySelector(".live-app .space-menu h1")).fontSize) < 20'), 'Space names must not inherit homepage headline typography');
        if (width === 390) browser('click', '.live-app .navigation-toggle');
        browser('click', '.live-app [aria-label="Create space"]');
        wait('!!document.querySelector(".space-dialog[open]")');
        screenshot(`homepage-create-space-${width}`);
        browser('press', 'Escape');
        assert.ok(evaluate('document.querySelector(".live-stage").hasAttribute("data-active")'), 'Dialog Escape must not exit the demo');
        browser('focus', '.live-app [aria-label="Manage fixture-channel"]');
        browser('press', 'Enter');
        browser('find', 'role', 'button', 'click', '--name', 'Channel settings', '--exact');
        wait('!!document.querySelector(".space-dialog[open]")');
        browser('press', 'Escape');
        if (width === 390) browser('click', '.live-app .channel-select');
      }
      assert.equal(evaluate('location.pathname + location.search'), '/', 'Embedded navigation must not rewrite the homepage URL');
      assert.ok(evaluate(`(() => {
        const exit = document.querySelector('.live-close').getBoundingClientRect();
        if (exit.top < 0 || exit.right > innerWidth || exit.bottom > innerHeight) return false;
        return [...document.querySelectorAll('.live-app button')].filter(b => b.getClientRects().length).every(b => {
          const r = b.getBoundingClientRect();
          return r.right <= exit.left || r.left >= exit.right || r.bottom <= exit.top || r.top >= exit.bottom;
        });
      })()`), 'Exit must not overlap app buttons');
      screenshot(`homepage-${guest ? 'guest' : 'account'}-${width}`);
      browser('click', '[aria-label="Exit demo"]');
      wait('!document.querySelector(".live-stage[data-active]")');
      assert.equal(evaluate('document.activeElement.className'), 'live-activator');
    }
  }
  console.log('PASS: homepage guest/account desktop and narrow layouts, local space navigation, create/manage dialogs, dialog Escape, exit and focus restoration (mock API/gateway).');
  }
} finally {
  try { browser('close'); } finally { rmSync(directory, { recursive: true, force: true }); }
}
