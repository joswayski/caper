// Disposable HTTP/WebSocket fixture + Chromium; not native/device acceptance.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const session = "pins-check";
const browser = (...args) => {
  const chrome = process.env.MESSAGE_TEST_CHROME ? ["--executable-path", process.env.MESSAGE_TEST_CHROME] : [];
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, ...chrome, ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const action = (name) => browser("find", "role", "button", "click", "--name", name, "--exact");
const waitCentered = (selector) => {
  evaluate("delete window.pinCenteredSince");
  // Stay visible across Virtuoso's deferred size/follow corrections, not just
  // for a single frame before a queued scroll-to-bottom overrides the jump.
  wait(`(() => {
    const r = document.querySelector(${JSON.stringify(selector)})?.getBoundingClientRect();
    if (!r || r.top <= 100 || r.bottom >= innerHeight - 100) {
      delete window.pinCenteredSince;
      return false;
    }
    window.pinCenteredSince ??= performance.now();
    return performance.now() - window.pinCenteredSince > 300;
  })()`);
};
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  assert.equal(evaluate("devicePixelRatio"), 2);
  browser("screenshot", `${artifacts}/${name}.png`);
};
const holdPinRequest = () =>
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => String(input).endsWith('/pin') && init?.method === 'PUT'
      ? new Promise((resolve, reject) => { window.releasePin = () => original(input, init).then(resolve, reject); })
      : original(input, init);
    window.restorePinFetch = () => { window.fetch = original; };
  })()`);
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
}
async function history(query = "", root) {
  const response = await fetch(
    `${api}/api/chat/channels/chan00000001/messages${root ? `/${root}/thread` : ""}${query}`,
    { headers: { authorization: "Bearer fixture-owner-token", connection: "close" } },
  );
  assert.equal(response.status, 200);
  return response.json();
}
const openPins = () => {
  browser("click", ".chat-pins-toggle");
  wait('!!document.querySelector(".chat-pins-dialog")');
};
const openMenu = () => {
  browser("hover", ".chat-pinned-message p");
  browser("click", ".chat-pinned-message .chat-message-actions-trigger");
  wait('!!document.querySelector(".chat-message-actions")');
};
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  await control({ reset: true });
  const initial = await history();
  const root = initial.messages[0];
  let old;
  for (let i = 0; i < 160; i++) {
    await control({ incomingMessage: { channelId: "chan00000001", text: `Context message ${i}` } });
    if (i === 45) old = (await history()).messages.at(-1);
  }
  await control({ incomingPin: { channelId: "chan00000001", messageId: old.id, userId: "member000001" } });
  assert.ok(!(await history()).messages.some((m) => m.id === old.id), "target is older than the latest page");
  const context = await history(`?around=${old.id}`);
  assert.equal(context.messages.length, 61);
  assert.equal(context.messages[30].id, old.id);
  assert.equal(context.messages[0].content.text, "TEST FIXTURE — Context message 15");
  assert.equal(context.messages.at(-1).content.text, "TEST FIXTURE — Context message 75");
  assert.equal(context.hasMore, true);
  assert.equal(context.hasNewer, true);

  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  wait(
    '!!document.querySelector(".chat-message-actions-trigger") && !document.querySelector(".chat-initial-messages")',
  );
  assert.equal(evaluate('document.querySelector(".chat-pins-toggle").getAttribute("aria-label")'), "Pins");
  assert.equal(
    evaluate('document.querySelector(".chat-pins-toggle").textContent.trim()'),
    "",
    "icon-only header control",
  );
  assert.equal(evaluate('!!document.querySelector(".chat-pins-toggle svg.lucide-pin")'), true);
  assert.equal(evaluate('!!document.querySelector(".chat-message .chat-pin-marker")'), false);
  openPins();
  assert.equal(evaluate('document.querySelector(".chat-pins-dialog").getAttribute("role")'), "dialog");
  assert.equal(evaluate('!!document.querySelector(".chat-history")'), true, "timeline remains mounted behind Pins");
  assert.equal(evaluate('document.querySelector(".chat-pinned-message p").textContent'), old.content.text);
  assert.equal(evaluate('document.querySelector(".chat-pins-dialog").textContent.includes("Unpin")'), false);
  assert.equal(
    evaluate(
      'document.querySelector(".chat-pinned-message .chat-message-actions-trigger").getAttribute("aria-haspopup")',
    ),
    "dialog",
  );
  browser("hover", ".chat-pins-dialog strong");
  if (process.env.MESSAGE_TEST_CHROME) {
    assert.equal(evaluate('matchMedia("(hover: hover) and (pointer: fine)").matches'), true);
    wait(
      'getComputedStyle(document.querySelector(".chat-pinned-message .chat-message-actions-trigger")).opacity === "0"',
    );
    browser("hover", ".chat-pinned-message p");
    wait(
      'getComputedStyle(document.querySelector(".chat-pinned-message .chat-message-actions-trigger")).opacity === "1"',
    );
  }
  screenshot("pins-desktop");
  assert.deepEqual(
    evaluate(`(() => {
    const style = getComputedStyle(document.querySelector('.chat-pinned-navigation button'));
    return [style.backgroundColor, style.borderTopWidth];
  })()`),
    ["rgba(0, 0, 0, 0)", "0px"],
    "Go is a quiet text action",
  );
  assert.notEqual(old.author.id, (await history()).pinnedMessages[0].pin.author.id);
  browser("hover", ".chat-pin-author");
  wait('document.querySelector(".chat-mention-card")?.textContent.includes("@maya")');
  assert.equal(evaluate('document.querySelector(".chat-mention-card strong").textContent'), "Maya");
  assert.equal(evaluate('!!document.querySelector(".chat-mention-card-message")'), true);
  screenshot("pins-desktop-profile");
  browser("press", "Escape");
  wait('!document.querySelector(".chat-mention-card")');
  assert.equal(evaluate('!!document.querySelector(".chat-pins-dialog")'), true, "profile Escape keeps Pins open");
  browser("mouse", "move", "30", "100");
  browser("hover", ".chat-pin-author");
  wait('!!document.querySelector(".chat-mention-card")');
  browser("mouse", "move", "30", "100");
  browser("mouse", "down");
  browser("mouse", "up");
  wait('!document.querySelector(".chat-mention-card")');
  assert.equal(
    evaluate('!!document.querySelector(".chat-pins-dialog")'),
    true,
    "profile outside dismissal keeps Pins open",
  );
  openMenu();
  assert.equal(evaluate('document.querySelector(".chat-message-actions").textContent.includes("Unpin message")'), true);
  screenshot("pins-desktop-actions");
  browser("press", "Escape");
  wait('!document.querySelector(".chat-message-actions")');
  assert.equal(evaluate('!!document.querySelector(".chat-pins-dialog")'), true, "Escape closes the nested menu first");
  browser("press", "Escape");
  wait('!document.querySelector(".chat-pins-dialog")');
  openPins();
  browser("mouse", "move", "30", "100");
  browser("mouse", "down");
  browser("mouse", "up");
  wait('!document.querySelector(".chat-pins-dialog")');
  openPins();
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.failContext = true;
    window.fetch = (input, init) => window.failContext && String(input).includes('?around=')
      ? Promise.resolve(Response.json({error:'TEST FIXTURE: context unavailable'}, {status:500})) : original(input, init);
  })()`);
  action("Go to message");
  wait('document.querySelector(".chat-pins-dialog [role=alert]")?.textContent.includes("context unavailable")');
  assert.equal(
    evaluate('!!document.querySelector(".chat-pins-dialog")'),
    true,
    "failed jump keeps Pins open for retry",
  );
  screenshot("pins-jump-error");
  evaluate("window.failContext = false");
  action("Go to message");
  const target = `[data-message-key="${old.clientMessageId}"]`;
  wait(
    `!document.querySelector('.chat-pins-dialog') && !!document.querySelector('${target}.chat-message-jump-target')`,
  );
  waitCentered(target);
  assert.equal(evaluate(`!!document.querySelector('${target} .chat-pin-marker')`), false);
  assert.equal(
    evaluate('!![...document.querySelectorAll("button")].find(b => b.textContent === "Back to latest")'),
    true,
  );
  screenshot("pins-jump-context");
  action("Back to latest");
  wait(`!document.querySelector('${target}') && !document.querySelector('.chat-initial-messages')`);
  wait(`(() => {
    const row = [...document.querySelectorAll('.chat-message p')].find(p => p.textContent === 'TEST FIXTURE — Context message 159');
    if (!row) return false;
    const r = row.getBoundingClientRect(); return r.top > 100 && r.bottom < innerHeight;
  })()`);
  openPins();
  action("Go to message");
  wait(`!!document.querySelector('${target}.chat-message-jump-target')`);
  waitCentered(target);
  browser("fill", "#chat-message", "Sent from pinned context");
  browser("press", "Enter");
  wait(
    `!document.querySelector('${target}') && ![...document.querySelectorAll('button')].some(b => b.textContent === 'Back to latest') && [...document.querySelectorAll('.chat-message:not(.chat-message-pending) p')].some(p => p.textContent.includes('Sent from pinned context'))`,
  );

  // Touch-sized browser layout: visible actions and outside dismissal, not a physical-device test.
  browser("set", "viewport", "390", "844", "2");
  if (evaluate('!!document.querySelector(".member-list-close")')) browser("click", ".member-list-close");
  openPins();
  assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
  screenshot("pins-narrow");
  // Synthetic touch input checks the hold handler and release-click guard;
  // Chromium viewport emulation is not physical-device acceptance.
  evaluate(`(() => {
    const button = document.querySelector('.chat-pin-author');
    const rect = button.getBoundingClientRect();
    button.dispatchEvent(new PointerEvent('pointerdown', {bubbles: true, pointerType: 'touch', pointerId: 777, isPrimary: true, clientX: rect.x + 5, clientY: rect.y + 5}));
  })()`);
  wait('document.querySelector(".chat-mention-card-drawer")?.textContent.includes("@maya")');
  evaluate(`(() => {
    document.querySelector('.chat-pin-author').dispatchEvent(new PointerEvent('pointerup', {bubbles: true, pointerType: 'touch', pointerId: 777, isPrimary: true}));
    document.querySelector('.chat-mention-card-message').dispatchEvent(new MouseEvent('click', {bubbles: true, cancelable: true}));
  })()`);
  assert.equal(
    evaluate('!!document.querySelector(".chat-mention-card-drawer") && !!document.querySelector(".chat-pins-dialog")'),
    true,
    "hold release must not activate Message",
  );
  screenshot("pins-narrow-profile");
  browser("press", "Escape");
  wait('!document.querySelector(".chat-mention-card")');
  openMenu();
  screenshot("pins-narrow-actions");
  // Preserve main's optimistic mutation coverage with the new menu/modal UI:
  // hold the request so the empty state cannot be caused by a fast server echo.
  holdPinRequest();
  action("Unpin message");
  wait('document.querySelector(".chat-pins-dialog")?.textContent.includes("No pinned messages.")');
  assert.equal((await history()).pinnedMessages.length, 1, "optimistic unpin has not reached the server");
  evaluate("(() => { window.restorePinFetch(); return window.releasePin(); })()");
  assert.equal((await history()).pinnedMessages.length, 0);
  screenshot("pins-empty");
  action("Close pins");

  // A pinned non-broadcast reply must navigate inside its thread, not the channel.
  let reply;
  for (let i = 0; i < 70; i++) {
    await control({ incomingReply: { rootId: root.id, text: `Thread context ${i}` } });
    if (i === 25) reply = (await history("", root.id)).messages.at(-1);
  }
  await control({ incomingPin: { channelId: "chan00000001", messageId: reply.id } });
  openPins();
  action("Go to message");
  wait(
    `!document.querySelector('.chat-pins-dialog') && document.querySelector('.chat-thread-panel')?.textContent.includes(${JSON.stringify(reply.content.text)})`,
  );
  wait(
    `(() => { const row = document.querySelector('.chat-thread-panel [data-message-id="${reply.id}"]'); if (!row) return false; const r = row.getBoundingClientRect(); return r.top > 0 && r.bottom < innerHeight; })()`,
  );
  assert.equal(
    evaluate('document.querySelector(".chat-thread-panel").textContent.includes("Load newer replies")'),
    true,
  );
  screenshot("pins-thread-context");
  browser("fill", "#chat-thread-reply", "Sent from pinned thread context");
  action("Send reply");
  wait(
    `document.querySelector('.chat-thread-panel')?.textContent.includes('Sent from pinned thread context') && !document.querySelector('.chat-thread-panel')?.textContent.includes('Load newer replies')`,
  );
  console.log(
    "PASS: icon-only Pins; quiet Go link; pinner hover profile by ID; profile outside/Escape dismissal; synthetic touch hold/release guard; no timeline pin notice/count; hover-menu optimistic unpin before server echo; outside/Escape dismissal; nested menu; failed jump retry; old-message context (30 before/30 after); centered target; back to newest message; sending from channel/thread context; narrow layout/unpin; pinned thread reply navigation.",
  );
} catch (error) {
  console.error(browser("snapshot", "-i").snapshot);
  browser("screenshot", "/tmp/caper-pins-test-failure.png");
  throw error;
} finally {
  try {
    browser("close");
  } catch {
    /* Preserve the original assertion failure. */
  }
}
