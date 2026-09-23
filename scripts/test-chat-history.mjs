// Browser regression with explicitly mocked history/WS, never production data.
// Start the web dev server, then: node scripts/test-chat-history.mjs
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const url = process.env.CHAT_TEST_WEB_URL ?? 'http://localhost:5174/spaces';
assert.ok(['localhost', '127.0.0.1'].includes(new URL(url).hostname), 'Use a disposable loopback preview');
const directory = mkdtempSync(join(tmpdir(), 'caper-history-'));
const init = join(directory, 'fixture.js');
const artifacts = process.env.CHAT_TEST_ARTIFACTS && resolve(process.env.CHAT_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });

function fixture() {
  if (location.protocol === 'about:') return;
  const base = 9007199254740992n;
  const author = { id: 'history-fixture', name: 'History fixture', isGuest: true };
  const ownAuthor = { id: 'local-fixture', name: 'Local fixture', isGuest: true };
  const message = (index, text, id, sender = author) => ({
    id: `message-${index}`, clientMessageId: id ?? `command-${index}`, channelId: 'general',
    seq: String(base + BigInt(index)), author: sender, createdAt: '2026-09-21T12:00:00Z',
    content: { version: 1, type: 'text', text: text ?? `Fixture message ${index}. ${index % 7 === 0 ? '\nA second line.\nAnd a third line.' : ''}${index % 11 === 0 ? 'Variable-length text for wrapping. '.repeat(20) : ''}` },
  });
  const messages = Array.from({ length: Number(new URL(location.href).searchParams.get('fixtureCount') ?? 3000) }, (_, i) => message(i + 1));
  const sockets = [];
  const control = window.chatHistoryFixture = {
    requests: [], completed: 0, failNext: false, holdNext: false, release: undefined,
    initialFrames: [],
    append(text = 'Fixture live arrival', id, sender) {
      const next = message(messages.length + 1, text, id, sender);
      messages.push(next);
      for (const socket of sockets) socket.frame({ type: 'message.created', channelId: 'general', seq: next.seq, message: next });
      return next;
    },
  };
  const page = (before = messages.length + 1) => {
    const end = before - 1, start = Math.max(0, end - 50);
    return { messages: messages.slice(start, end), cursor: String(base + BigInt(messages.length)), hasMore: start > 0 };
  };
  const originalFetch = window.fetch;
  window.fetch = async (input, options) => {
    const path = new URL(typeof input === 'string' ? input : input.url, location.href);
    if (path.pathname === '/api/account/me') return new Response(null, { status: 401 });
    if (path.pathname === '/api/chat/general') return Response.json({ ...page(), space: { id: 'fixture', name: 'History fixture' }, channel: { id: 'general', name: 'General' } });
    if (path.pathname === '/api/chat/session') return Response.json({ token: 'local-test-only', author: ownAuthor });
    if (path.pathname.endsWith('/typing')) return new Response(null, { status: 204 });
    if (path.pathname === '/api/chat/channels/general/messages') {
      if (options?.method === 'POST') {
        const body = JSON.parse(options.body);
        await new Promise(resolve => setTimeout(resolve, 250));
        return Response.json(control.append(body.text, body.clientMessageId, ownAuthor));
      }
      control.requests.push(path.searchParams.get('before'));
      const response = page(Number(BigInt(path.searchParams.get('before')) - base));
      if (control.holdNext) {
        control.holdNext = false;
        await new Promise(resolve => { control.release = resolve; });
        control.release = undefined;
      }
      await new Promise(resolve => setTimeout(resolve, 80));
      control.completed++;
      if (control.failNext) { control.failNext = false; return Response.json({ error: 'Fixture history outage' }, { status: 503 }); }
      return Response.json(response);
    }
    return originalFetch(input, options);
  };
  const OriginalSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    constructor(address, protocols) {
      super();
      if (!String(address).includes('/api/chat/events')) return new OriginalSocket(address, protocols);
      sockets.push(this);
      setTimeout(() => this.frame({ type: 'ready', cursor: new URL(address).searchParams.get('after') }), 0);
    }
    frame(event) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) })); }
    close() { const index = sockets.indexOf(this); if (index >= 0) sockets.splice(index, 1); }
  };
  window.localStorage.removeItem('caper.chat.session');
  // Check painted content, not merely the absence of a loading label. Virtuoso
  // used to expose an empty list during its initial scroll-to-bottom frames.
  const sampleInitialPaint = () => {
    const region = document.querySelector('.chat-messages[aria-busy="false"]');
    if (region && messages.length) {
      const bounds = region.getBoundingClientRect();
      control.initialFrames.push([...region.querySelectorAll('.chat-message')].some(row => {
        const rect = row.getBoundingClientRect();
        return row.checkVisibility({ visibilityProperty: true }) && rect.bottom > bounds.top && rect.top < bounds.bottom;
      }));
      if (!document.querySelector('.chat-initial-messages')) return;
    }
    requestAnimationFrame(sampleInitialPaint);
  };
  requestAnimationFrame(sampleInitialPaint);
}

writeFileSync(init, `(${fixture.toString()})()`);
const args = ['--session', 'chat-history-test', '--init-script', init];
function browser(...command) {
  const output = execFileSync('agent-browser', [...args, ...command, '--json'], { encoding: 'utf8', timeout: 60000 });
  const result = JSON.parse(output);
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = source => browser('eval', source).result;
const wait = expression => browser('wait', '--fn', expression);
const settle = () => evaluate('new Promise(r => setTimeout(r, 300))');
const screenshot = name => { if (artifacts) browser('screenshot', '--full', join(artifacts, `${name}.png`)); };
const metrics = () => evaluate(`(() => { const s = document.querySelector('.chat-scroller'); return { rows: document.querySelectorAll('.chat-message').length, bottom: s.scrollHeight - s.clientHeight - s.scrollTop, top: s.scrollTop }; })()`);
const anchor = () => evaluate(`(() => {
  const top = document.querySelector('.chat-scroller').getBoundingClientRect().top;
  const row = [...document.querySelectorAll('.chat-message')].find(row => row.getBoundingClientRect().bottom > top);
  return { key: row.dataset.messageKey, offset: row.getBoundingClientRect().top - top };
})()`);
const anchorOffset = key => evaluate(`document.querySelector('[data-message-key="${key}"]').getBoundingClientRect().top - document.querySelector('.chat-scroller').getBoundingClientRect().top`);

try {
  browser('open', 'about:blank');
  browser('set', 'viewport', '1280', '800', '2');
  browser('open', url);
  wait('!!document.querySelector(".chat-message") && !document.querySelector(".chat-offline")');
  evaluate('document.fonts.ready.then(() => true)');
  wait('(() => { const s = document.querySelector(".chat-scroller"); return s.scrollHeight - s.clientHeight - s.scrollTop < 2; })()');
  settle();
  assert.ok(metrics().rows < 35);
  assert.ok(evaluate('chatHistoryFixture.initialFrames.length > 0 && chatHistoryFixture.initialFrames.every(Boolean)'), 'History must be visible on every ready frame, including virtualizer positioning');
  assert.ok(metrics().bottom < 2, JSON.stringify(metrics()));
  assert.equal(evaluate('chatHistoryFixture.requests.length'), 0, 'Initial positioning must not fetch older pages');
  screenshot('chat-history-latest');

  evaluate('chatHistoryFixture.holdNext = true; document.querySelector(".chat-scroller").scrollTop = 0');
  wait('typeof chatHistoryFixture.release === "function"');
  const before = anchor();
  evaluate('chatHistoryFixture.append()');
  settle();
  assert.ok(Math.abs(anchorOffset(before.key) - before.offset) < 2, 'Live append moved the reader');
  evaluate('chatHistoryFixture.release()');
  wait('chatHistoryFixture.completed === 1');
  settle();
  assert.ok(Math.abs(anchorOffset(before.key) - before.offset) < 2, 'Prepending variable-height rows moved the reader');
  assert.equal(evaluate('chatHistoryFixture.requests.length'), 1);

  evaluate('chatHistoryFixture.failNext = true; document.querySelector(".chat-scroller").scrollTop = 0');
  wait('document.querySelector(".chat-history").textContent.includes("Couldn’t")');
  const attempts = evaluate('chatHistoryFixture.requests.length');
  settle();
  assert.equal(evaluate('chatHistoryFixture.requests.length'), attempts, 'Failure must not create an automatic retry loop');
  screenshot('chat-history-retry');
  browser('click', '.chat-history button');
  wait('chatHistoryFixture.completed === 3');
  settle();
  assert.equal(evaluate('chatHistoryFixture.requests.at(-1)'), evaluate('chatHistoryFixture.requests.at(-2)'), 'Retry must use the same cursor');

  // Fetch every older page via scrolling, never clicking the fallback button.
  for (let batch = 0; batch < 6; batch++) {
    evaluate(`(async () => {
      for (let i = 0; i < 10; i++) {
        if (document.querySelector('.chat-history').textContent.includes('Beginning')) break;
        const completed = chatHistoryFixture.completed;
        document.querySelector('.chat-scroller').scrollTop = 0;
        const deadline = performance.now() + 3000;
        while (chatHistoryFixture.completed === completed) {
          if (performance.now() > deadline) throw Error('Scroll failed to load page: ' + JSON.stringify({ completed, requests: chatHistoryFixture.requests.length, top: document.querySelector('.chat-scroller').scrollTop }));
          await new Promise(r => setTimeout(r, 20));
        }
        await new Promise(r => setTimeout(r, 300));
      }
    })()`);
  }
  wait('document.querySelector(".chat-history").textContent.includes("Beginning")');
  evaluate('document.querySelector(".chat-scroller").scrollTop = 0');
  settle();
  assert.ok(evaluate('!!document.querySelector("[data-message-key=command-1]")'));
  assert.ok(metrics().rows < 40, 'Thousands of loaded messages must not become thousands of DOM rows');
  assert.equal(evaluate('chatHistoryFixture.requests.length'), 60, '59 older pages plus one failed attempt');
  screenshot('chat-history-beginning');

  browser('set', 'viewport', '390', '844', '2');
  settle();
  assert.ok(metrics().rows < 40);
  assert.equal(evaluate('document.documentElement.scrollWidth <= innerWidth'), true);
  evaluate('document.querySelector(".chat-scroller").scrollTop = 500');
  settle();
  screenshot('chat-history-narrow');

  evaluate('document.querySelector(".chat-scroller").focus()');
  browser('press', 'End');
  wait('(() => { const s = document.querySelector(".chat-scroller"); return s.scrollHeight - s.clientHeight - s.scrollTop < 2 && !!document.querySelector("[data-message-key=command-3001]"); })()');
  settle();
  evaluate('chatHistoryFixture.append("Fixture arrival at the bottom")');
  settle();
  assert.ok(metrics().bottom < 2, 'Following live messages at bottom must still work: ' + JSON.stringify(metrics()));
  evaluate('document.querySelector(".chat-scroller").scrollTop = 0');
  settle();
  browser('fill', '#chat-message', 'Fixture optimistic message');
  browser('press', 'Enter');
  wait('!!document.querySelector("[data-message-key=command-3002]") && !document.querySelector(".chat-message-pending") && document.querySelector("#chat-message").value === ""');
  settle();
  assert.equal(evaluate('[...document.querySelectorAll(".chat-message p")].filter(p => p.textContent === "Fixture optimistic message").length'), 1);
  assert.ok(metrics().bottom < 2);
  console.log(`PASS: 3,000-message history; 50-message cursor pages; stable prepend/live anchors; ${metrics().rows} mounted rows at latest; retry; narrow layout; optimistic reconciliation after sending from history.`);

  const emptyUrl = new URL(url);
  emptyUrl.searchParams.set('fixtureCount', '0');
  browser('open', emptyUrl.toString());
  wait('document.querySelector(".chat-state")?.textContent.includes("No messages yet.")');
  screenshot('chat-history-empty');
  browser('fill', '#chat-message', 'First fixture message');
  browser('press', 'Enter');
  wait('document.querySelectorAll(".chat-message").length === 1 && !document.querySelector(".chat-message-pending")');
  assert.equal(evaluate('document.querySelector(".chat-message p").textContent'), 'First fixture message');
  assert.equal(evaluate('chatHistoryFixture.requests.length'), 0);
  console.log('PASS: empty history mounts a virtual list on first send without duplication or pagination.');
  assert.ok(evaluate('chatHistoryFixture.initialFrames.length > 0 && chatHistoryFixture.initialFrames.every(Boolean)'), 'First-message mount must not flash empty');
} finally {
  try { browser('close'); } finally { rmSync(directory, { recursive: true, force: true }); }
}
