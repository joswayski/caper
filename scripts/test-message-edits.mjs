// Disposable loopback fixture + Chromium. Not native/device or live-service acceptance.
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
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "edits-check", ...args, "--json"], {
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
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  assert.equal(evaluate("devicePixelRatio"), 2);
  browser("screenshot", `${artifacts}/${name}.png`);
};
const headers = { authorization: "Bearer fixture-owner-token", connection: "close" };
const control = async (body) => {
  const result = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(result.status, 200);
};
const history = async () => (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers })).json();
const row = (message, surface = ".chat-panel") => `${surface} [data-message-key="${message.clientMessageId}"]`;
const edit = (message, surface) => {
  if (!surface) browser("scroll", "up", "1000", "--selector", ".chat-scroller");
  wait(
    `(() => { const button = document.querySelector('${row(message, surface)} .chat-message-actions-trigger'); if (!button) return false; const rect = button.getBoundingClientRect(); return document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)?.closest('button') === button; })()`,
  );
  browser("click", `${row(message, surface)} .chat-message-actions-trigger`);
  wait('!!document.querySelector(".chat-message-actions")');
  action("Edit message");
  wait('!!document.querySelector(".chat-edit-dialog[open] #chat-edit-text")');
  assert.equal(evaluate("document.activeElement.id"), "chat-edit-text");
};
const save = (text) => {
  browser("fill", "#chat-edit-text", text);
  browser("press", "Control+Enter");
  wait('!document.querySelector("#chat-edit-text")');
};

try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  await control({ reset: true });
  const initial = await history();
  const root = initial.messages.find((message) => message.author.id === "owner0000001");
  assert.ok(root);
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  wait(
    `!!document.querySelector('${row(root)} .chat-message-actions-trigger') && !document.querySelector('.chat-initial-messages')`,
  );
  const other = initial.messages.find((message) => message.author.id !== root.author.id);
  browser("click", `${row(other)} .chat-message-actions-trigger`);
  wait('!!document.querySelector(".chat-message-actions")');
  assert.equal(
    evaluate(
      'Array.from(document.querySelectorAll(".chat-message-actions button")).some(button => button.textContent === "Edit message")',
    ),
    false,
  );
  browser("press", "Escape");
  edit(root);
  save("TEST FIXTURE — Meet Friday at 9.");
  wait(`document.querySelector('${row(root)} p')?.textContent === 'TEST FIXTURE — Meet Friday at 9.'`);
  assert.equal(evaluate(`document.querySelector('${row(root)} .chat-edited').textContent`), "edited");

  // Pin projections support both editing and retained-history access.
  browser("click", `${row(root)} .chat-message-actions-trigger`);
  wait('!!document.querySelector(".chat-message-actions")');
  action("Pin message");
  wait('document.querySelector(".chat-pins-toggle").getAttribute("aria-label") === "Pins, 1"');
  action("Pins, 1");
  wait('!!document.querySelector(".chat-pinned-message")');
  action("Edit message");
  save("TEST FIXTURE — Meet Saturday at 11.");
  wait('document.querySelector(".chat-pinned-message p")?.textContent === "TEST FIXTURE — Meet Saturday at 11."');
  screenshot("message-edit-pins-desktop");
  browser("click", ".chat-pinned-message .chat-edited");
  wait('document.querySelectorAll("#chat-version-select option").length === 3');
  const latest = 'document.querySelector(".chat-version-comparison diffs-container").shadowRoot';
  wait(`${latest}?.querySelector('[data-content] [data-line]')?.textContent.includes('Meet Friday')`);
  assert.equal(
    evaluate(`${latest}.querySelector('[data-deletions] [data-content] [data-line]').textContent`),
    "TEST FIXTURE — Meet Friday at 9.",
  );
  assert.equal(
    evaluate(`${latest}.querySelector('[data-additions] [data-content] [data-line]').textContent`),
    "TEST FIXTURE — Meet Saturday at 11.",
  );
  assert.deepEqual(
    evaluate(
      `Array.from(${latest}.querySelectorAll('[data-deletions] [data-diff-span]')).map(span => span.textContent)`,
    ),
    ["Friday", "9"],
  );
  assert.deepEqual(
    evaluate(
      `Array.from(${latest}.querySelectorAll('[data-additions] [data-diff-span]')).map(span => span.textContent)`,
    ),
    ["Saturday", "11"],
  );
  assert.equal(evaluate(`${latest}.querySelector('pre').getAttribute('data-diff-type')`), "split");
  browser("select", "#chat-version-select", "2");
  wait(
    '!!document.querySelector(".chat-version-older diffs-container")?.shadowRoot?.querySelector("[data-content] [data-line]")',
  );
  assert.equal(evaluate('document.querySelectorAll(".chat-version-comparison").length'), 2);
  assert.equal(
    evaluate(`${latest}.querySelector('[data-additions] [data-content] [data-line]').textContent`),
    "TEST FIXTURE — Meet Saturday at 11.",
    "Browsing older versions must not replace the latest pair",
  );
  screenshot("message-history-desktop");
  browser("select", "#chat-version-select", "1");
  assert.equal(evaluate('document.querySelector(".chat-version-original").textContent'), root.content.text);
  browser("set", "viewport", "390", "844", "2");
  assert.equal(
    evaluate(
      'document.documentElement.scrollWidth <= innerWidth && document.querySelector(".chat-version-dialog").scrollWidth <= document.querySelector(".chat-version-dialog").clientWidth',
    ),
    true,
  );
  screenshot("message-history-narrow");
  browser("set", "viewport", "1440", "900", "2");
  browser("press", "Escape");
  action("Messages");

  // Real stale-revision conflict: keep the draft; explicit reload discards it.
  browser("set", "viewport", "390", "844", "2");
  if (evaluate('!!document.querySelector(".member-list-close")')) browser("click", ".member-list-close");
  edit(root);
  browser("fill", "#chat-edit-text", "TEST FIXTURE — Unsaved narrow draft.");
  await control({
    incomingEdit: { channelId: root.channelId, messageId: root.id, text: "TEST FIXTURE — Meet Thursday at 10." },
  });
  action("Save changes");
  wait('!!document.querySelector(".chat-edit-dialog [role=alert]")');
  assert.equal(evaluate('document.querySelector("#chat-edit-text").value'), "TEST FIXTURE — Unsaved narrow draft.");
  assert.equal(evaluate("document.activeElement.id"), "chat-edit-text");
  assert.equal(evaluate("document.documentElement.scrollWidth <= innerWidth"), true);
  screenshot("message-edit-conflict-narrow");
  action("Discard draft and load latest");
  wait(
    'document.querySelector("#chat-edit-text").value === "TEST FIXTURE — Meet Thursday at 10." && !document.querySelector(".chat-edit-dialog [role=alert]")',
  );
  action("Cancel");
  browser("set", "viewport", "1440", "900", "2");

  browser("click", `${row(root)} .chat-reply-thread`);
  wait(
    '!!document.querySelector("#chat-thread-reply") && !document.querySelector(".chat-thread-messages[aria-busy=true]")',
  );
  for (const broadcast of [false, true]) {
    if (broadcast) browser("click", ".chat-thread-send-row input");
    browser(
      "fill",
      "#chat-thread-reply",
      broadcast ? "TEST FIXTURE — Broadcast reply." : "TEST FIXTURE — Hidden reply.",
    );
    browser("click", ".chat-thread-send-row button");
    wait(
      '!document.querySelector(".chat-thread-panel .chat-message-pending") && document.querySelector("#chat-thread-reply").value === ""',
    );
  }
  const thread = await (
    await fetch(`${api}/api/chat/channels/${root.channelId}/messages/${root.id}/thread`, { headers })
  ).json();
  const hidden = thread.messages.find((message) => !message.broadcast);
  const broadcast = thread.messages.find((message) => message.broadcast);
  const summary = evaluate(`document.querySelector('${row(root)} .chat-thread-summary strong').textContent`);
  edit(hidden, ".chat-thread-panel");
  save("TEST FIXTURE — Corrected hidden reply.");
  assert.equal(evaluate(`!!document.querySelector('${row(hidden)}')`), false);
  wait(
    `document.querySelector('${row(hidden, ".chat-thread-panel")} p')?.textContent === 'TEST FIXTURE — Corrected hidden reply.'`,
  );
  edit(broadcast, ".chat-thread-panel");
  save("TEST FIXTURE — Corrected shared broadcast.");
  wait(
    `['.chat-panel', '.chat-thread-panel'].every(surface => document.querySelector(surface + ' [data-message-key="${broadcast.clientMessageId}"] p')?.textContent === 'TEST FIXTURE — Corrected shared broadcast.')`,
  );
  assert.equal(evaluate(`document.querySelector('${row(root)} .chat-thread-summary strong').textContent`), summary);
  browser("click", `${row(broadcast, ".chat-thread-panel")} .chat-edited`);
  wait('!!document.querySelector("#chat-version-select")');
  browser("press", "Escape");
  assert.equal(
    evaluate('!!document.querySelector(".chat-thread-panel")'),
    true,
    "History Escape must not close its thread",
  );
  browser("press", "Escape");

  // Pagination and recovery use explicitly labelled disposable fixture responses.
  for (let index = 0; index < 51; index++)
    await control({
      incomingEdit: {
        channelId: root.channelId,
        messageId: root.id,
        text: `TEST FIXTURE — Schedule revision ${index}.`,
      },
    });
  wait(
    `document.querySelector('${row(root)} .chat-edited')?.getAttribute('aria-label') === 'Message history, version 55'`,
  );
  await control({
    failure: {
      path: `/api/chat/channels/${root.channelId}/messages/${root.id}/versions`,
      status: 503,
      persistent: true,
      error: "TEST FIXTURE: versions unavailable.",
    },
  });
  browser("click", `${row(root)} .chat-edited`);
  wait('!!document.querySelector(".chat-version-dialog [role=alert]")');
  await control({ clearFailures: true });
  action("Retry");
  wait('document.querySelectorAll("#chat-version-select option").length === 50');
  action("Load older versions");
  wait('document.querySelectorAll("#chat-version-select option").length === 55');
  browser("select", "#chat-version-select", "1");
  assert.equal(evaluate('document.querySelector(".chat-version-original").textContent'), root.content.text);
  browser("press", "Escape");

  // A rejected edit preserves the draft and does not mutate the message.
  edit(root);
  await control({
    failure: {
      path: `/api/chat/channels/${root.channelId}/messages/${root.id}`,
      method: "PUT",
      status: 403,
      error: "TEST FIXTURE: editing forbidden.",
    },
  });
  browser("fill", "#chat-edit-text", "TEST FIXTURE — Must not save.");
  action("Save changes");
  wait('!!document.querySelector(".chat-edit-dialog [role=alert]")');
  assert.equal(evaluate('document.querySelector("#chat-edit-text").value'), "TEST FIXTURE — Must not save.");
  assert.equal(
    (await history()).messages.find((message) => message.id === root.id).content.text,
    "TEST FIXTURE — Schedule revision 50.",
  );
  action("Cancel");
  console.log(
    "Message edits browser checks passed: ownership, save, pins, diffs, narrow conflict, hidden/broadcast replies, independent summary, history retry/paging and rejected saves.",
  );
} finally {
  try {
    browser("close");
  } catch {}
}
