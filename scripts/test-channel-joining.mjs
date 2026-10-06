// Explicitly labelled, disposable browser mocks; never contacts a real API/SFU.
// CHANNEL_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-channel-joining.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const url = process.env.CHANNEL_TEST_WEB_URL ?? 'http://localhost:5174/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname));
const artifacts = process.env.CHANNEL_TEST_ARTIFACTS && resolve(process.env.CHANNEL_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(join(tmpdir(), 'caper-channels-'));
const init = join(directory, 'fixture.js');
function fixture() {
  if (location.protocol === 'about:') return;
  const account = { id: 'member123456', username: 'member', displayName: 'TEST FIXTURE member' };
  const space = { id: 'space1234567', name: 'TEST FIXTURE · Studio', ownerId: 'owner1234567' };
  const general = { id: 'first1234567', spaceId: space.id, name: 'general', private: false };
  const design = { id: 'other1234567', spaceId: space.id, name: 'design', private: false };
  const privateChannel = { id: 'third1234567', spaceId: space.id, name: 'planning', private: true };
  const empty = new URL(location.href).searchParams.has('empty');
  let state = JSON.parse(sessionStorage.getItem('channel-test-state') ?? 'null') ?? { joined: empty ? [] : [general.id], invited: true, granted: false };
  const save = () => sessionStorage.setItem('channel-test-state', JSON.stringify(state));
  const control = window.channelFixture = { requests: [], microphones: 0, fail: false, state };
  if (navigator.mediaDevices) navigator.mediaDevices.getUserMedia = async () => { control.microphones++; throw new Error('TEST FIXTURE: microphone disabled'); };
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
    if (!path.startsWith('/api/')) return originalFetch(input, options);
    const method = options.method ?? 'GET';
    control.requests.push({ path, method });
    const error = (status, message) => Response.json({ error: message }, { status });
    const channels = [general, design, ...(state.granted ? [privateChannel] : [])].map(channel => ({ ...channel, joined: state.joined.includes(channel.id) }));
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/spaces') return Response.json({ spaces: [space], invitations: [], limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 } });
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels, members: [{ ...account, owner: false }], channelInvitations: state.invited ? [{ channel: { ...privateChannel, joined: false }, inviter: { username: 'owner', displayName: 'TEST FIXTURE Owner' } }] : [] });
    const membership = path.match(/\/channels\/([^/]+)\/membership$/);
    if (membership) {
      if (control.fail) return error(503, 'TEST FIXTURE: try again');
      const channel = channels.find(channel => channel.id === membership[1]);
      if (!channel) return error(404, 'resource not found');
      state.joined = state.joined.filter(id => id !== channel.id);
      if (method === 'POST') state.joined.push(channel.id);
      else if (channel.private) state.granted = false;
      save();
      return method === 'DELETE' ? new Response(null, { status: 204 }) : Response.json({ ...channel, joined: true });
    }
    if (path.endsWith(`/channels/${privateChannel.id}/invitation`)) {
      if (control.fail) return error(503, 'TEST FIXTURE: try again');
      if (!state.invited) return error(404, 'resource not found');
      state.invited = false;
      if (method === 'POST') { state.granted = true; state.joined.push(privateChannel.id); }
      save();
      return method === 'DELETE' ? new Response(null, { status: 204 }) : Response.json({ ...privateChannel, joined: true });
    }
    const history = path.match(/\/chat\/channels\/([^/]+)\/messages$/);
    if (history) {
      const channel = channels.find(channel => channel.id === history[1]);
      if (!channel) return error(404, 'resource not found');
      if (method === 'POST') return error(503, 'TEST FIXTURE: sends disabled');
      return Response.json({ space, channel, cursor: '1', hasMore: false, messages: [{ id: `message-${channel.id}`, channelId: channel.id, seq: '1', clientMessageId: '00000000-0000-4000-8000-000000000001', createdAt: '2026-10-01T14:00:00Z', author: { id: 'owner1234567', name: 'TEST FIXTURE Owner', isGuest: false }, reactions: [{ emoji: '👍', authorIds: [account.id, 'owner1234567'] }], reactionSeq: '1', content: { version: 1, type: 'text', text: `TEST FIXTURE — ${channel.name} conversation. Previewing does not join this channel.` } }] });
    }
    if (path === '/api/chat/session') return Response.json({ token: 'fixture-only', author: { id: account.id, name: account.displayName, isGuest: false } });
    if (path.endsWith('/media/status')) return Response.json({ enabled: false });
    return error(503, 'TEST FIXTURE: disabled endpoint');
  };
}
writeFileSync(init, `(${fixture.toString()})()`);
const args = ['--session', 'channel-joining', '--init-script', init];
const browser = (...command) => {
  const result = JSON.parse(execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 60000 }));
  assert.ok(result.success, result.error); return result.data;
};
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);
const screenshot = name => { if (artifacts) browser('screenshot', `${artifacts}/${name}.png`); };
try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  browser('open', url);
  wait('!!document.querySelector(".space-menu summary")');
  assert.equal(evaluate('document.querySelector("#space-channel-list").textContent.includes("design")'), false);
  browser('click', '.space-menu summary');
  browser('find', 'role', 'button', 'click', '--name', 'Browse channels', '--exact');
  wait('!!document.querySelector(".channel-directory")');
  browser('fill', '.channel-directory input', 'des');
  browser('find', 'role', 'button', 'click', '--name', 'Preview #design', '--exact');
  wait('!!document.querySelector(".channel-preview")');
  assert.equal(evaluate('document.querySelector("#chat-message") !== null'), false);
  assert.equal(evaluate('document.querySelector(".channel-preview span").previousElementSibling.textContent'), 'Preview');
  assert.equal(evaluate('document.querySelector(".channel-preview span").textContent'), 'Join #design to interact with people here');
  assert.equal(evaluate('document.querySelector(".channel-preview span strong").textContent'), '#design');
  assert.ok(evaluate('Number(getComputedStyle(document.querySelector(".channel-preview span strong")).fontWeight) > Number(getComputedStyle(document.querySelector(".channel-preview span")).fontWeight)'));
  assert.equal(evaluate('getComputedStyle(document.querySelector(".channel-preview span strong")).display'), 'inline');
  assert.equal(evaluate('channelFixture.state.joined.includes("other1234567")'), false);
  assert.equal(evaluate('channelFixture.requests.some(r => r.path.includes("other1234567/media") || r.path.endsWith("membership"))'), false);
  assert.equal(evaluate('channelFixture.microphones'), 0);
  wait('!!document.querySelector(".chat-reaction")');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-reaction,.chat-add-reaction")].every(button => button.disabled || button.getAttribute("aria-disabled") === "true")'), true);
  assert.equal(evaluate('document.querySelector(".chat-reaction").textContent'), '2');
  screenshot('public-channel-preview-1280');
  evaluate('channelFixture.fail = true');
  browser('find', 'role', 'button', 'click', '--name', 'Join channel', '--exact');
  wait('!!document.querySelector(".channel-preview [role=alert]")');
  assert.equal(evaluate('channelFixture.state.joined.includes("other1234567")'), false);
  evaluate('channelFixture.fail = false');
  browser('find', 'role', 'button', 'click', '--name', 'Join channel', '--exact');
  wait('!!document.querySelector("#chat-message") && channelFixture.state.joined.includes("other1234567")');
  assert.equal(evaluate('channelFixture.microphones'), 0);
  browser('reload');
  wait('document.querySelector("#space-channel-list").textContent.includes("design")');
  wait('!document.querySelector(".chat-initial-messages") && !!document.querySelector(\'.chat-reaction:not([aria-disabled="true"])\')');
  browser('find', 'first', '.chat-reaction', 'click');
  wait('!!document.querySelector(".chat-send-error")');
  const reactionWrites = evaluate('channelFixture.requests.filter(r => r.path.endsWith("/reactions") && r.method === "PUT").length');
  assert.equal(reactionWrites, 1, 'Joined channel can attempt a reaction; mock intentionally rejects to expose retry');
  browser('find', 'first', '.chat-add-reaction', 'click');
  wait('!!document.querySelector(".chat-reaction-picker")');
  evaluate('channelFixture.state.joined = channelFixture.state.joined.filter(id => id !== "other1234567"); window.dispatchEvent(new Event("focus"))');
  wait('!!document.querySelector(".channel-preview") && !document.querySelector("#chat-message")');
  assert.equal(evaluate('document.querySelector("#space-channel-list").textContent.includes("design")'), false);
  assert.equal(evaluate('[...document.querySelectorAll(".chat-reaction,.chat-add-reaction")].every(button => button.disabled || button.getAttribute("aria-disabled") === "true")'), true);
  assert.equal(evaluate('document.querySelector(".chat-reaction-picker") === null'), true);
  assert.equal(evaluate('[...document.querySelectorAll("button")].filter(button => button.textContent === "Retry reaction").every(button => button.disabled)'), true);
  evaluate('[...document.querySelectorAll(".chat-reaction,.chat-add-reaction, .chat-send-error button")].filter(button => button.textContent !== "Dismiss").forEach(button => button.click())');
  assert.equal(evaluate('channelFixture.requests.filter(r => r.path.endsWith("/reactions") && r.method === "PUT").length'), reactionWrites);
  browser('find', 'role', 'button', 'click', '--name', 'Join channel', '--exact');
  wait('!!document.querySelector("#chat-message") && channelFixture.state.joined.includes("other1234567")');
  assert.equal(evaluate('document.querySelector(".chat-heading").textContent.includes("Leave channel")'), false, 'Leaving is not a chat-header action');
  browser('click', '[aria-label="Manage design"]');
  assert.equal(evaluate('document.querySelector(".channel-menu[open]").textContent.includes("Channel settings")'), false, 'Members can leave without gaining owner settings');
  browser('find', 'role', 'button', 'click', '--name', 'Leave channel', '--exact');
  wait('!!document.querySelector(".leave-channel-consent")');
  browser('find', 'role', 'button', 'click', '--name', 'Cancel', '--exact');
  assert.equal(evaluate('document.activeElement.getAttribute("aria-label")'), 'Manage design', 'Cancelling restores focus to the channel menu');
  assert.ok(evaluate('channelFixture.state.joined.includes("other1234567")'), 'Opening the menu and cancelling never changes membership');
  browser('click', '[aria-label="Manage design"]');
  screenshot('leave-channel-menu-1280');
  browser('find', 'role', 'button', 'click', '--name', 'Leave channel', '--exact');
  browser('find', 'role', 'button', 'click', '--name', 'Leave channel', '--exact');
  wait('!channelFixture.state.joined.includes("other1234567") && !!document.querySelector(".channel-preview")');
  browser('click', '.pending-channel-invite');
  wait('!!document.querySelector(".channel-invitation-consent")');
  assert.equal(evaluate('channelFixture.requests.some(r => r.path.includes("third1234567/messages") || r.path.includes("third1234567/media"))'), false);
  assert.ok(evaluate('document.querySelector(".channel-invitation-consent").textContent.includes("TEST FIXTURE Owner")'));
  screenshot('private-channel-consent-1280');
  browser('find', 'role', 'button', 'click', '--name', 'Accept invitation', '--exact');
  wait('channelFixture.state.granted && !!document.querySelector("#chat-message")');
  browser('click', '[aria-label="Manage planning"]');
  browser('find', 'role', 'button', 'click', '--name', 'Leave channel', '--exact');
  assert.ok(evaluate('document.querySelector(".leave-channel-consent").textContent.includes("another invitation")'));
  browser('find', 'role', 'button', 'click', '--name', 'Leave channel', '--exact');
  wait('!channelFixture.state.granted && !document.querySelector("#space-channel-list").textContent.includes("planning")');

  browser('set', 'viewport', '390', '844', '2');
  evaluate('sessionStorage.removeItem("channel-test-state")');
  browser('open', `${url}?empty`);
  wait('!!document.querySelector(".empty-channel")');
  browser('find', 'role', 'button', 'click', '--name', 'Browse channels', '--exact');
  wait('!!document.querySelector(".channel-directory")');
  assert.equal(evaluate('document.documentElement.scrollWidth > innerWidth'), false);
  screenshot('browse-channels-390');
  browser('find', 'role', 'button', 'click', '--name', 'Preview #design', '--exact');
  wait('!!document.querySelector(".channel-preview")');
  assert.equal(evaluate('document.documentElement.scrollWidth > innerWidth'), false);
  wait('!!document.querySelector(".chat-reaction")');
  assert.equal(evaluate('[...document.querySelectorAll(".chat-reaction,.chat-add-reaction")].every(button => button.disabled || button.getAttribute("aria-disabled") === "true")'), true);
  screenshot('public-channel-preview-390');
  browser('find', 'role', 'button', 'click', '--name', 'Browse', '--exact');
  browser('click', '.pending-channel-invite');
  wait('!!document.querySelector(".channel-invitation-consent")');
  screenshot('private-channel-consent-390');
  browser('find', 'role', 'button', 'click', '--name', 'Decline', '--exact');
  wait('!channelFixture.state.invited');
  assert.equal(evaluate('channelFixture.requests.some(r => r.path.includes("third1234567/messages"))'), false);
  assert.equal(evaluate('channelFixture.microphones'), 0);
  console.log('PASS: joined-only sidebar, search, read-only preview, explicit join, retry, reload persistence, focus reconciliation, public/private leave, private accept/decline privacy, no microphone; desktop + narrow Chromium.');
} finally {
  browser('close');
  rmSync(directory, { recursive: true, force: true });
}
