// Disposable loopback fixture + Chromium. Narrow/touch checks are not native-device acceptance.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (...args) => {
  const chrome = process.env.MESSAGE_TEST_CHROME ? ["--executable-path", process.env.MESSAGE_TEST_CHROME] : [];
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "threads-check", ...chrome, ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  assert.equal(evaluate("devicePixelRatio"), 2);
  browser("screenshot", `${artifacts}/${name}.png`);
};
const row = (id) => `[data-message-key="${id}"]`;
const channel = (id) => `.chat-panel ${row(id)}`;
const thread = (id) => `.chat-thread-panel ${row(id)}`;
const headers = { authorization: "Bearer fixture-owner-token", connection: "close" };
const history = async () => (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers })).json();
const close = () => {
  browser("press", "Escape");
  wait('!document.querySelector(".chat-thread-panel")');
};
const open = (id) => {
  browser("focus", `${channel(id)} .chat-reply-thread`);
  browser("click", `${channel(id)} .chat-reply-thread`);
  wait(
    '!!document.querySelector(".chat-thread-panel") && !document.querySelector(".chat-thread-messages[aria-busy=true]")',
  );
};
const reply = (text) => {
  browser("fill", "#chat-thread-reply", text);
  browser("click", ".chat-thread-send-row button");
  wait(
    '!document.querySelector(".chat-thread-panel .chat-message-pending") && document.querySelector("#chat-thread-reply").value === ""',
  );
};

let socket;
try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: "close" } })).json()).fixture, true);
  const reset = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify({ reset: true }),
  });
  assert.equal(reset.status, 200);
  const initial = await history();
  const root = initial.messages.at(-1);
  const other = initial.messages[1];
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  wait(
    `!!document.querySelector('${channel(root.clientMessageId)}') && !document.querySelector('.chat-initial-messages')`,
  );
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadRequests = [];
    window.fetch = (input, init) => {
      if (String(input).includes('/thread')) window.threadRequests.push(String(input));
      return original(input, init);
    };
  })()`);
  if (process.env.MESSAGE_TEST_CHROME) {
    assert.equal(evaluate('matchMedia("(hover: hover) and (pointer: fine)").matches'), true);
    browser("hover", "#chat-heading");
    assert.equal(
      evaluate(
        `getComputedStyle(document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread')).opacity`,
      ),
      "0",
    );
    browser("focus", `${channel(root.clientMessageId)} .chat-reply-thread`);
    wait(
      `getComputedStyle(document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread')).opacity === '1'`,
    );
    assert.equal(
      evaluate(
        `getComputedStyle(document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread')).opacity`,
      ),
      "1",
    );
    assert.deepEqual(
      evaluate(
        `['.chat-reply-thread', '.chat-add-reaction', '.chat-message-actions-trigger'].map(selector => { const rect = document.querySelector('${channel(root.clientMessageId)} ' + selector).getBoundingClientRect(); return [rect.width, rect.height]; })`,
      ),
      [
        [24, 24],
        [24, 24],
        [24, 24],
      ],
    );
  }
  open(root.clientMessageId);
  assert.equal(evaluate("document.activeElement.id"), "chat-thread-reply");
  assert.equal(evaluate('document.querySelector(".chat-thread-send-row input").checked'), false);
  assert.equal(
    evaluate(
      `document.querySelector('${channel(root.clientMessageId)}').classList.contains('chat-message-thread-active')`,
    ),
    true,
  );
  assert.equal(evaluate('document.querySelector(".chat-thread-panel").hasAttribute("aria-modal")'), false);
  assert.equal(
    evaluate('document.querySelector(".chat-thread-status").textContent'),
    "No replies yet. Start the thread.",
  );
  screenshot("threads-desktop-empty");
  reply("TEST FIXTURE — Keep the layout discussion here. 🙂");
  wait(
    `document.querySelector('${channel(root.clientMessageId)} .chat-thread-summary strong')?.textContent === '1 reply'`,
  );
  assert.equal(
    evaluate('document.querySelectorAll(".chat-panel .chat-message").length'),
    initial.messages.length,
    "Thread-only replies stay out of the channel",
  );
  assert.equal(
    evaluate(`document.querySelectorAll('${channel(root.clientMessageId)} .chat-thread-avatars > span').length`),
    1,
  );
  browser("click", ".chat-thread-send-row input");
  reply("TEST FIXTURE — This update is also visible in the channel.");
  const page = await (
    await fetch(`${api}/api/chat/channels/chan00000001/messages/${root.id}/thread`, { headers })
  ).json();
  assert.equal(page.messages.length, 2);
  const broadcast = page.messages.at(-1);
  assert.equal(broadcast.broadcast, true);
  wait(
    `!!document.querySelector('${channel(broadcast.clientMessageId)}') && !!document.querySelector('${thread(broadcast.clientMessageId)}')`,
  );
  assert.equal((await history()).messages.filter((message) => message.id === broadcast.id).length, 1);
  browser("click", `${thread(broadcast.clientMessageId)} .chat-message-actions-trigger`);
  wait('!!document.querySelector(".chat-message-actions")');
  browser("find", "role", "button", "click", "--name", "React with 🎉", "--exact");
  wait(
    `document.querySelector('${thread(broadcast.clientMessageId)} .chat-reaction')?.getAttribute('aria-pressed') === 'true' && document.querySelector('${channel(broadcast.clientMessageId)} .chat-reaction')?.getAttribute('aria-pressed') === 'true'`,
  );
  screenshot("threads-desktop");
  browser("fill", "#chat-thread-reply", "Saved draft for Alex");
  const requestCount = evaluate("window.threadRequests.length");
  const positions = () =>
    evaluate(
      `Array.from(document.querySelectorAll('.chat-thread-panel .chat-message')).map(row => row.getBoundingClientRect().top)`,
    );
  const before = positions();
  browser("click", `${channel(root.clientMessageId)} .chat-thread-summary`);
  browser("click", `${channel(root.clientMessageId)} .chat-thread-summary`);
  assert.equal(evaluate("window.threadRequests.length"), requestCount, "repeated clicks cannot fetch again");
  assert.deepEqual(positions(), before, "repeated clicks cannot shift the displayed replies");
  assert.equal(
    evaluate('!!document.querySelector(".chat-thread-skeleton, .chat-thread-messages[aria-busy=true]")'),
    false,
  );
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "Saved draft for Alex");
  if (process.env.MESSAGE_TEST_CHROME) {
    browser("hover", `${channel(other.clientMessageId)} .chat-reply-thread`);
    wait(`window.threadRequests.some(url => url.includes('/${other.id}/thread'))`);
    assert.equal(
      evaluate(`!!document.querySelector('${thread(root.clientMessageId)}')`),
      true,
      "hover does not change the open thread",
    );
  }
  open(other.clientMessageId);
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "");
  browser("fill", "#chat-thread-reply", "Saved draft for Maya");
  open(root.clientMessageId);
  assert.equal(evaluate('document.querySelector("#chat-thread-reply").value'), "Saved draft for Alex");
  close();
  assert.equal(
    evaluate(
      `document.activeElement === document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread')`,
    ),
    true,
    "Closing restores trigger focus",
  );
  browser("click", `${channel(broadcast.clientMessageId)} .chat-thread-context`);
  wait(`!!document.querySelector('${thread(root.clientMessageId)}')`);
  assert.equal(
    evaluate(`window.threadRequests.filter(url => url.includes('/${root.id}/thread')).length`),
    1,
    "closing and reopening uses the loaded thread",
  );
  assert.equal(
    evaluate('document.querySelectorAll(".chat-thread-panel .chat-message").length'),
    3,
    "A broadcast opens its original root, never a nested thread",
  );
  close();

  // Explicitly labelled response mock exercises loading/error/retry without an external outage.
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadFetch = original;
    window.fetch = (input, init) => String(input).includes('/thread') ? new Promise(resolve => { window.releaseThread = () => resolve(Response.json({ error: 'TEST FIXTURE: thread unavailable' }, { status: 503 })); }) : original(input, init);
  })()`);
  browser("click", `${channel(initial.messages[0].clientMessageId)} .chat-reply-thread`);
  wait('document.querySelector(".chat-thread-messages").getAttribute("aria-busy") === "true"');
  assert.equal(
    evaluate('document.querySelector(".chat-thread-skeleton").getAttribute("aria-label")'),
    "Loading thread replies",
  );
  assert.equal(evaluate('document.querySelector(".chat-thread-status")'), null, "no text loading row");
  screenshot("threads-desktop-loading");
  evaluate("window.releaseThread()");
  wait('document.querySelector(".chat-thread-status[role=alert]")?.textContent.includes("TEST FIXTURE")');
  screenshot("threads-desktop-error");
  evaluate("window.fetch = window.threadFetch");
  browser("click", ".chat-thread-status[role=alert] button");
  wait(
    '!document.querySelector(".chat-thread-status[role=alert]") && !document.querySelector(".chat-thread-messages[aria-busy=true]")',
  );
  close();

  browser("find", "role", "button", "click", "--name", "Show member list", "--exact");
  wait('!!document.querySelector("#space-member-list")');
  browser("set", "viewport", "390", "844", "2");
  // Explicitly open it above: members now start closed on desktop too.
  // The open desktop member list becomes a mobile overlay on resize.
  browser("click", ".member-list-close");
  wait('!document.querySelector("#space-member-list")');
  socket = new WebSocket(browser("get", "cdp-url").cdpUrl);
  await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
  let nextID = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data),
      request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
  });
  const cdp = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++nextID;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
  const { targetInfos } = await cdp("Target.getTargets");
  const target = targetInfos.find((target) => target.type === "page" && target.url.startsWith(web));
  const { sessionId } = await cdp("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  assert.equal(
    evaluate(
      `(() => { const reply = document.querySelector('${channel(root.clientMessageId)} .chat-reply-thread').getBoundingClientRect(); const more = document.querySelector('${channel(root.clientMessageId)} .chat-message-actions-trigger').getBoundingClientRect(); return reply.right <= more.left && reply.width >= 44 && more.width >= 44; })()`,
    ),
    true,
    "Touch controls must not overlap",
  );
  screenshot("threads-mobile-channel");
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.threadFetch = original;
    window.fetch = (input, init) => String(input).includes('/thread') ? new Promise((resolve, reject) => { window.releaseThread = () => original(input, init).then(resolve, reject); }) : original(input, init);
  })()`);
  browser("click", `${channel(initial.messages[2].clientMessageId)} .chat-reply-thread`);
  wait('!!document.querySelector(".chat-thread-skeleton")');
  screenshot("threads-mobile-loading");
  evaluate("window.fetch = window.threadFetch; window.releaseThread()");
  wait('!document.querySelector(".chat-thread-messages[aria-busy=true]")');
  close();
  open(root.clientMessageId);
  wait('document.querySelector(".chat-thread-panel").getAttribute("aria-modal") === "true"');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), true);
  assert.deepEqual(
    evaluate(
      '(() => { const rect = document.querySelector(".chat-thread-panel").getBoundingClientRect(); return [rect.x, rect.y, rect.width, rect.height]; })()',
    ),
    [0, 0, 390, 844],
  );
  screenshot("threads-mobile");
  evaluate("history.back()");
  wait('!document.querySelector(".chat-thread-panel")');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), false);
  open(root.clientMessageId);
  browser("click", ".chat-thread-back");
  wait('!document.querySelector(".chat-thread-panel")');
  assert.equal(evaluate('document.querySelector(".chat-panel").inert'), false);
  console.log(
    "PASS: repeated clicks make zero requests and zero reply-position changes; hover/focus prefetch, cached reopening, cold-load skeletons, loading/error/retry, isolated replies, shared broadcast reactions, drafts, focus restoration, touch targets, full-screen dialog, inert background and mobile Back.",
  );
} finally {
  socket?.close();
  browser("close");
}
