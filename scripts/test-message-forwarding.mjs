// Disposable loopback fixture + Chromium. These checks are not native-device acceptance.
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
    execFileSync("agent-browser", ["--session", "forward-check", ...args, "--json"], {
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
const request = async (path, body, who = "owner", method = body ? "POST" : "GET", token) => {
  const response = await fetch(api + path, {
    method,
    headers: {
      authorization: `Bearer fixture-${who}-token`,
      connection: "close",
      ...(body ? { "content-type": "application/json" } : {}),
      ...(token ? { "x-caper-chat-token": token } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  return { status: response.status, value: await response.json() };
};
const control = (body) => request("/__fixture/control", body);

try {
  assert.equal((await request("/health")).value.fixture, true);
  assert.equal((await control({ reset: true })).status, 200);
  const source = (await request("/api/chat/channels/chan00000003/messages")).value.messages[0];
  await control({
    incomingEdit: { messageId: source.id, text: "TEST FIXTURE — Live discussion from private planning." },
  });
  await control({ incomingReply: { rootId: source.id, text: "An existing reply before sharing." } });
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector(".channel-select")');
  browser("find", "role", "button", "click", "--name", "planning", "--exact");
  const sourceRow = `[data-message-key="${source.clientMessageId}"]`;
  wait(
    `!!document.querySelector('${sourceRow}') && !document.querySelector('.chat-initial-messages') && document.querySelector('#chat-heading')?.textContent.includes('planning')`,
  );
  browser("focus", `${sourceRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  assert.equal(
    evaluate(
      'Array.from(document.querySelectorAll(".chat-copy-actions button")).filter(button => button.textContent === "Edit message").length',
    ),
    1,
    "Source author retains the edit action",
  );
  browser("find", "role", "button", "click", "--name", "Forward message", "--exact");
  wait('document.querySelectorAll(".chat-forward-destinations input").length > 0');
  browser("click", ".chat-forward-destinations label:nth-child(2) input");
  assert.equal(
    evaluate(
      'document.querySelector(".chat-forward-destinations input:checked").closest("label").querySelector("strong").textContent',
    ),
    "# general",
  );
  browser("fill", ".chat-forward-body textarea", "TEST FIXTURE — Check this live conversation.");
  screenshot("forward-picker-desktop");
  // Explicitly labelled lost-response mock: the real fixture commits before the response is dropped.
  evaluate(`(() => {
    window.forwardFetch = window.fetch.bind(window); let drop = true;
    window.fetch = async (input, init) => {
      const response = await window.forwardFetch(input, init);
      if (drop && String(input).endsWith('/forwards') && init?.method === 'POST') { drop = false; throw new TypeError('TEST FIXTURE: response lost after commit'); }
      return response;
    };
  })()`);
  browser("click", ".chat-forward-send");
  wait('document.querySelector(".chat-forward-error")?.textContent.includes("Not confirmed")');
  assert.equal(evaluate('document.querySelector(".chat-forward-body textarea").disabled'), true);
  screenshot("forward-retry-desktop");
  browser("click", ".chat-forward-send");
  wait('!document.querySelector(".chat-forward-dialog")');
  const forwards = (await request("/api/chat/channels/chan00000001/messages")).value.messages.filter(
    (message) => message.forward,
  );
  assert.equal(forwards.length, 1, "Retry does not duplicate a committed forward");
  const wrapper = forwards[0];
  assert.equal(wrapper.forward.message.id, source.id);
  assert.equal(wrapper.forward.message.thread.replyCount, 1);
  browser("find", "role", "button", "click", "--name", "general", "--exact");
  wait('!!document.querySelector(".chat-forward-card") && !document.querySelector(".chat-initial-messages")');
  const wrapperRow = `[data-message-key="${wrapper.clientMessageId}"]`;
  browser("focus", `${wrapperRow} .chat-message-actions-trigger`);
  browser("press", "Enter");
  assert.equal(
    evaluate(
      'Array.from(document.querySelectorAll(".chat-copy-actions button")).filter(button => /Edit message|Message history/.test(button.textContent)).length',
    ),
    0,
    "Even the wrapper author has no edit/history controls",
  );
  screenshot("forward-read-only-actions");
  browser("press", "Escape");
  const session = (await request("/api/chat/session", { name: "Fixture Owner" })).value;
  assert.equal(
    (
      await request(
        `/api/chat/channels/${wrapper.channelId}/messages/${wrapper.id}`,
        { text: "Forbidden wrapper edit", expectedRevision: 1 },
        "owner",
        "PUT",
        session.token,
      )
    ).status,
    404,
  );
  browser("cookies", "set", "caper_fixture", "other", "--url", web, "--path", "/", "--sameSite", "Lax");
  evaluate("localStorage.clear()");
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector(".chat-forward-card") && !document.querySelector(".chat-initial-messages")');
  assert.equal((await request("/api/chat/channels/chan00000003/messages", undefined, "other")).status, 404);
  assert.equal(
    (await request(`/api/chat/channels/${source.channelId}/messages/${source.id}/versions`, undefined, "other")).status,
    404,
  );
  browser("click", ".chat-forward-card button");
  wait('document.querySelector(".chat-forward-body h3")?.textContent === "1 reply"');
  assert.equal(
    evaluate('document.querySelectorAll(".chat-forward-dialog input, .chat-forward-dialog textarea").length'),
    0,
    "Original conversation is read-only",
  );
  assert.equal(evaluate('document.querySelectorAll(".chat-forward-dialog .chat-forward-original").length'), 2);
  const saved = await request(
    `/api/chat/channels/${source.channelId}/messages/${source.id}`,
    { text: "TEST FIXTURE — The original changed after sharing.", expectedRevision: 2 },
    "owner",
    "PUT",
    session.token,
  );
  assert.equal(saved.status, 200);
  assert.equal(saved.value.revision, 3);
  await control({ incomingReaction: { channelId: source.channelId, messageId: source.id, emoji: "👀" } });
  await control({ incomingReply: { rootId: source.id, text: "A future reply shared automatically." } });
  wait(
    'document.querySelector(".chat-forward-body h3")?.textContent === "2 replies" && document.querySelector(".chat-forward-body").textContent.includes("A future reply") && document.querySelector(".chat-forward-body").textContent.includes("The original changed") && !!document.querySelector(".chat-forward-reactions")',
  );
  screenshot("forward-conversation-desktop");
  browser("set", "viewport", "390", "844", "2");
  screenshot("forward-conversation-mobile");
  assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
  browser("press", "Escape");
  wait('!document.querySelector(".chat-forward-dialog")');
  if (evaluate('!!document.querySelector(".member-list-close")')) browser("click", ".member-list-close");
  browser("click", `${wrapperRow} .chat-reply-thread`);
  wait('!!document.querySelector("#chat-thread-reply")');
  browser("fill", "#chat-thread-reply", "TEST FIXTURE — Replying only in the destination.");
  browser("click", ".chat-thread-send-row button");
  wait('document.querySelector(".chat-thread-summary strong")?.textContent === "1 reply"');
  const destinationThread = (
    await request(`/api/chat/channels/chan00000001/messages/${wrapper.id}/thread`, undefined, "other")
  ).value;
  assert.equal(destinationThread.messages.length, 1);
  assert.equal(destinationThread.root.forward.message.thread.replyCount, 2);
  assert.equal(
    (await request(`/api/chat/channels/chan00000003/messages/${source.id}/thread`)).value.messages.length,
    2,
  );
  screenshot("forward-destination-thread-mobile");
  console.log(
    "Forwarding browser checks passed: retry deduplication, destination-only live edits/reactions/replies, read-only originals, independent destination thread, desktop/narrow layouts.",
  );
} finally {
  browser("close");
}
