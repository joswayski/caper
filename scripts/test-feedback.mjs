// Explicitly mocked browser regression. Does not access accounts or production data.
// FEEDBACK_TEST_ARTIFACTS=.amp/in/artifacts node scripts/test-feedback.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const base = process.env.FEEDBACK_TEST_URL ?? 'http://localhost:5174/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(base).hostname));
const artifacts = process.env.FEEDBACK_TEST_ARTIFACTS && resolve(process.env.FEEDBACK_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const temporary = mkdtempSync(join(tmpdir(), 'feedback-ui-'));
function fixture() {
  if (location.protocol === 'about:') return;
  const owner = new URL(location.href).searchParams.has('owner');
  const account = { id: owner ? 'owner1234567' : 'alice1234567', username: owner ? 'jose' : 'alice', displayName: owner ? 'Jose' : 'Alice' };
  const space = { id: 'feedback1234', name: 'Feedback', ownerId: 'owner1234567', feedback: true };
  const publicChannels = ['general', 'ideas', 'bugs'].map((name, i) => ({ id: `public00000${i}`, spaceId: space.id, name, private: false, latestSeq: '0', unread: false }));
  const privateChannel = { id: 'private00001', spaceId: space.id, name: owner ? 'Alice @alice' : 'Chat with Jose', private: true, feedbackUserId: 'alice1234567', latestSeq: '8', unread: true };
  const older = { ...privateChannel, id: 'private00002', name: 'Bob @bob', feedbackUserId: 'bobby1234567' };
  const control = window.feedbackFixture = { reads: [], creates: 0, created: owner, fail: false, pages: [], messages: [] };
  const history = channel => ({ space, channel, messages: channel.private ? [{ id: 'msg000000001', channelId: channel.id, seq: '7', author: { id: 'owner1234567', name: 'Jose', isGuest: false }, content: { version: 1, type: 'text', text: 'Thanks for the feedback. Can you tell me a little more?' }, createdAt: '2026-09-29T12:00:00Z', clientMessageId: 'fixture-message' }, ...control.messages] : [], cursor: channel.private ? '7' : '0', hasMore: false });
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href);
    const path = url.pathname;
    if (!path.startsWith('/api/')) return nativeFetch(input, options);
    if (path === '/api/account/me') return Response.json(account);
    if (path === '/api/chat/session') return Response.json({ token: 'fixture-token', author: { id: account.id, name: account.displayName, isGuest: false } });
    if (path === '/api/spaces') return Response.json({ spaces: [space], limits: { ownedSpaces: 1, totalSpaces: 1, channelsPerSpace: 3 } });
    if (path.endsWith('/feedback')) {
      control.creates++;
      if (control.fail) return Response.json({ error: 'Test-only creation failure' }, { status: 503 });
      control.created = true;
      return Response.json(privateChannel);
    }
    if (path.endsWith('/read')) {
      control.reads.push({ path, ...JSON.parse(options.body) });
      return new Response(null, { status: 204 });
    }
    if (path === `/api/spaces/${space.id}`) {
      control.pages.push(url.search);
      const selected = url.searchParams.get('channel');
      const olderPage = url.searchParams.has('beforeFeedback');
      return Response.json({ space, channels: [...publicChannels, ...(control.created ? [olderPage ? older : privateChannel] : []), ...(selected === older.id && !olderPage ? [older] : [])], members: [], nextFeedbackBefore: owner && !olderPage ? privateChannel.id : null });
    }
    if (path.endsWith('/messages')) {
      const channel = [...publicChannels, privateChannel, older].find(c => path.includes(c.id));
      if (options.method === 'POST') {
        const body = JSON.parse(options.body);
        const message = { id: 'msg000000002', channelId: channel.id, seq: '8', author: { id: account.id, name: account.displayName, isGuest: false }, content: { version: 1, type: 'text', text: body.text }, createdAt: new Date().toISOString(), clientMessageId: body.clientMessageId };
        control.messages.push(message);
        return Response.json(message);
      }
      return Response.json(history(channel));
    }
    return Response.json({ error: 'Unavailable in explicit UI fixture' }, { status: 503 });
  };
  window.WebSocket = class extends EventTarget {
    constructor() { super(); queueMicrotask(() => this.frame({ type: 'hello', idleTimeoutSeconds: 600, serverTime: Date.now() })); }
    frame(value) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(value) })); }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === 'heartbeat') this.frame({ type: 'heartbeat' });
      if (request.type === 'subscribe') {
        this.frame({ type: 'event', id: request.id, event: request.kind === 'chat' ? { type: 'ready', cursor: request.after ?? '0' } : { type: 'snapshot', participants: [], members: [], revision: 1 } });
        this.frame({ type: 'subscribed', id: request.id });
      }
    }
    close() {}
  };
}
const init = join(temporary, 'fixture.js');
writeFileSync(init, `(${fixture.toString()})()`);
function browser(...command) {
  const result = JSON.parse(execFileSync('agent-browser', ['--session', 'feedback-test', '--init-script', init, ...command, '--json'], { encoding: 'utf8', timeout: 60_000 }));
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = source => browser('eval', source).result;
const wait = source => browser('wait', '--fn', source);
const screenshot = name => { if (artifacts) browser('screenshot', `${artifacts}/${name}.png`); };
try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '900', '2');
  browser('open', `${base}?space=feedback1234`);
  wait('!!document.querySelector(".feedback-actions")');
  assert.equal(evaluate('document.querySelectorAll(".channel-select").length'), 3);
  assert.equal(evaluate('document.querySelector(".space-menu, .channel-manage, .member-list-toggle")'), null);
  assert.equal(evaluate('document.querySelector(".add-space").disabled'), false, 'Feedback must not consume quota');
  assert.equal(evaluate('feedbackFixture.creates'), 0, 'Reading or prefetching must not create a conversation');
  screenshot('feedback-public-desktop');
  evaluate('feedbackFixture.fail = true');
  browser('click', '.feedback-actions button');
  wait('!!document.querySelector(".feedback-actions [role=alert]")');
  screenshot('feedback-create-error');
  evaluate('feedbackFixture.fail = false');
  browser('click', '.feedback-actions button');
  wait('document.querySelector(".chat-channel-title")?.textContent.includes("chat with jose")');
  wait('feedbackFixture.reads.some(r => r.seq === "7")');
  assert.ok(evaluate('feedbackFixture.reads.every(r => r.seq === "7")'), 'Read displayed seq 7, not metadata head 8');
  browser('fill', '.chat-composer textarea', 'The mobile navigation was hard to find.');
  browser('press', 'Enter');
  wait('feedbackFixture.messages.length === 1');
  wait('feedbackFixture.reads.some(r => r.seq === "8")');
  screenshot('feedback-private-desktop');
  browser('set', 'viewport', '390', '844', '2');
  browser('click', '.navigation-toggle');
  wait('!!document.querySelector(".navigation-open")');
  assert.ok(evaluate('document.documentElement.scrollWidth <= innerWidth'));
  screenshot('feedback-private-narrow');
  browser('set', 'viewport', '1280', '900', '2');
  browser('open', `${base}?space=feedback1234&owner=1`);
  wait('!!document.querySelector(".feedback-unread")');
  assert.equal(evaluate('document.querySelector(".channel-manage, .space-menu")'), null);
  assert.equal(evaluate('feedbackFixture.reads.length'), 0, 'Opening shared channel must not read private inbox');
  screenshot('feedback-owner-inbox');
  browser('click', '.feedback-actions button:last-of-type');
  wait('document.querySelectorAll(".channel-select").length === 5');
  assert.ok(evaluate('feedbackFixture.pages.some(p => p.includes("beforeFeedback=private00001"))'));
  browser('click', '#space-channel-list > li:last-child .channel-select');
  wait('document.querySelector(".chat-channel-title")?.textContent.includes("bob @bob")');
  assert.ok(evaluate('feedbackFixture.pages.some(p => p.includes("channel=private00002"))'));
  console.log('Feedback browser checks passed: shared/private, failure+retry, send, displayed-only reads, quotas, owner paging, desktop/narrow layout. Mocked API; not live delivery proof.');
} finally {
  browser('close');
  rmSync(temporary, { recursive: true, force: true });
}
