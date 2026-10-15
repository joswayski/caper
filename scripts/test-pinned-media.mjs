// Real web UI + disposable native-parity-fixture API. No production data or SFU.
// Run with the fixture on 3001 and web dev server on 5174 (or MESSAGE_TEST_* URLs).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { Worker } from "node:worker_threads";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.MESSAGE_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (...args) => {
  const chrome = process.env.MESSAGE_TEST_CHROME ? ["--executable-path", process.env.MESSAGE_TEST_CHROME] : [];
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "pin-media", ...chrome, ...args, "--json"], {
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
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
}
async function history(root) {
  const response = await fetch(`${api}/api/chat/channels/chan00000001/messages${root ? `/${root}/thread` : ""}`, {
    headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
  });
  assert.equal(response.status, 200);
  return response.json();
}
const openPins = () => {
  if (evaluate("innerWidth <= 760")) browser("click", ".chat-title-menu > summary");
  action("Pins");
  wait('!!document.querySelector(".chat-pins-dialog")');
  wait(
    'document.getAnimations().every(a => a.playState !== "running" || a.effect?.getTiming().iterations === Infinity)',
  );
};
let mediaServer;
try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: "close" } })).json()).fixture, true);
  await control({ reset: true });
  const root = (await history()).messages[0];
  await control({ incomingMessage: { channelId: "chan00000001", text: "Pinned GIF preview" } });
  const message = (await history()).messages.at(-1);
  // Tiny looping GIF with full-size 16:9 preview metadata, so narrow widths must scale it down.
  const gif =
    "R0lGODlhQAAkAPAAAGN6Q/Ht5CH/C05FVFNDQVBFMi4wAwEAAAAh+QQAMgAAACwAAAAAQAAkAAACXYyPqYvgD6OckNmbqN4Re8uF2kdm4lmVKsoC6tqebxnL81eLN55zu9fz/S7BzZBYpBxBycmS0XQ+FVHJlFpNXQ9Z7TbQDYvH5LL5jE6r1+y2+w2Py+f0uv2Oz+vrBQAh+QQAMgAAACwAAAAAQAAkAAACU4SPqcvtD6OctNqLs968+w+G4kiW5omm6sq27gvH8kwzwY3n+s7nZA8M6n7CYo9oTOKQyiSzWXxCg9LpcWQ1VrNDLJfq/V5FYuC2fBanv2tuO1sAADs=";
  // Serve media off-thread while the synchronous browser CLI runs.
  mediaServer = new Worker(
    `
    const { parentPort, workerData } = require('node:worker_threads');
    const server = require('node:http').createServer((_request, response) => {
      response.writeHead(200, { 'content-type': 'image/gif' });
      response.end(Buffer.from(workerData, 'base64'));
    });
    server.listen(0, '127.0.0.1', () => parentPort.postMessage(server.address().port));
  `,
    { eval: true, workerData: gif },
  );
  const mediaPort = await new Promise((resolve, reject) => {
    mediaServer.once("message", resolve);
    mediaServer.once("error", reject);
  });
  await control({
    incomingEdit: {
      channelId: message.channelId,
      messageId: message.id,
      text: message.content.text,
      attachments: [
        {
          id: "fixturepingif01",
          kind: "image",
          contentType: "image/gif",
          name: "TEST FIXTURE landscape.gif",
          size: Buffer.from(gif, "base64").length,
          width: 640,
          height: 360,
          status: "ready",
          url: `http://127.0.0.1:${mediaPort}/test.gif`,
        },
      ],
    },
  });
  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  const row = `.chat-message[data-message-id="${message.id}"]`;
  wait(
    `!!document.querySelector('${row} .chat-gif img')?.complete && !document.querySelector('.chat-initial-messages')`,
  );
  browser("hover", `${row} p`);
  browser("click", `${row} .chat-message-actions-trigger`);
  action("Pin message");
  wait('document.querySelector(".chat-action-status")?.textContent.includes("Message pinned")');
  assert.deepEqual(
    (await history()).pinnedMessages.map((m) => m.id),
    [message.id],
    "pins the chosen GIF, not its neighbor",
  );
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    browser("reload");
    wait('!!document.querySelector(".chat-pins-toggle") && !document.querySelector(".chat-initial-messages")');
    wait(`document.querySelector('${row} .chat-pin-author')?.textContent === 'Pinned by you'`);
    browser("mouse", "move", "0", "0");
    assert.equal(
      evaluate(`getComputedStyle(document.querySelector('${row}')).backgroundColor`),
      "rgba(228, 199, 106, 0.06)",
      "pinned messages keep the gold wash without hovering",
    );
    browser("hover", `${row} p`);
    assert.equal(
      evaluate(`getComputedStyle(document.querySelector('${row}')).backgroundColor`),
      "rgba(228, 199, 106, 0.06)",
      "hover does not replace the pinned highlight",
    );
    assert.equal(
      evaluate(`(() => {
        const row = document.querySelector('${row}');
        const marker = row.querySelector('.chat-pin-marker').getBoundingClientRect();
        const text = row.querySelector('p').getBoundingClientRect();
        return marker.bottom <= text.top && Math.abs(marker.left - text.left) < 1;
      })()`),
      true,
      "pin attribution sits above the message and aligns with its text",
    );
    screenshot(`pins-timeline-${width}`);
    openPins();
    wait('!!document.querySelector(".chat-pinned-message .chat-gif img")?.complete');
    assert.equal(
      evaluate('document.querySelector(".chat-pinned-message .chat-pin-author").textContent'),
      "Pinned by you",
    );
    screenshot(`pins-gif-${width}`);
    const bounds = evaluate(`(() => {
      const card = document.querySelector('.chat-pinned-message');
      const media = card.querySelector('.chat-media').getBoundingClientRect();
      const text = card.querySelector('p').getBoundingClientRect();
      const go = card.querySelector('.chat-pinned-navigation').getBoundingClientRect();
      return { width: media.width, height: media.height, left: media.left, textLeft: text.left,
        bottom: media.bottom, goTop: go.top, right: media.right, cardRight: card.getBoundingClientRect().right };
    })()`);
    console.log(`GIF at ${width}px viewport:`, bounds);
    assert.ok(bounds.width > 200, "GIF is not squeezed into the avatar column");
    assert.ok(Math.abs(bounds.width / bounds.height - 16 / 9) < 0.02, "GIF keeps its aspect ratio");
    assert.ok(Math.abs(bounds.left - bounds.textLeft) < 1, "media aligns with message text");
    assert.ok(bounds.goTop >= bounds.bottom && bounds.right < bounds.cardRight, "navigation stays below fitted media");
    browser("click", ".chat-pinned-message .chat-pin-author");
    wait('document.querySelector(".chat-mention-card strong")?.textContent === "Fixture Owner"');
    action("Close profile");
    await control({ incomingPin: { channelId: message.channelId, messageId: message.id, active: false } });
    wait(`!document.querySelector('${row} .chat-pin-marker')`);
    assert.notEqual(
      evaluate(`getComputedStyle(document.querySelector('${row}')).backgroundColor`),
      "rgba(228, 199, 106, 0.06)",
      "unpin removes the persistent timeline highlight",
    );
    await control({ incomingPin: { channelId: message.channelId, messageId: message.id, userId: "member000001" } });
    wait('document.querySelector(".chat-pinned-message .chat-pin-author")?.textContent === "Pinned by Maya"');
    assert.equal(evaluate(`document.querySelector('${row} .chat-pin-author').textContent`), "Pinned by Maya");
    screenshot(`pins-other-${width}`);
    browser("click", ".chat-pinned-message .chat-media");
    wait('!!document.querySelector(".chat-viewer")');
    action("Close viewer");
    assert.equal(
      evaluate('!!document.querySelector(".chat-pins-dialog")'),
      true,
      "media does not trigger the card jump",
    );
    browser("click", ".chat-pinned-message .chat-author-name");
    wait('!!document.querySelector(".chat-mention-card")');
    action("Close profile");
    assert.equal(
      evaluate('!!document.querySelector(".chat-pins-dialog")'),
      true,
      "profile does not trigger the card jump",
    );
    browser("click", ".chat-pinned-message > p");
    wait(`!document.querySelector('.chat-pins-dialog') && !!document.querySelector('${row}.chat-message-jump-target')`);
    assert.equal(evaluate(`document.querySelector('${row} .chat-pin-author').textContent`), "Pinned by Maya");
    await control({ incomingPin: { channelId: message.channelId, messageId: message.id, active: false } });
    await control({ incomingPin: { channelId: message.channelId, messageId: message.id, userId: "owner0000001" } });
  }
  // Reload without the original in the latest page; failures must leave Pins retryable.
  for (let i = 0; i < 60; i++)
    await control({ incomingMessage: { channelId: message.channelId, text: `Later message ${i}` } });
  browser("set", "viewport", "1280", "900", "2");
  browser("reload");
  wait('!!document.querySelector(".chat-pins-toggle") && !document.querySelector(".chat-initial-messages")');
  openPins();
  evaluate(`(() => {
    const original = window.fetch.bind(window);
    window.failContext = true;
    window.fetch = (input, init) => window.failContext && String(input).includes('?around=')
      ? Promise.resolve(Response.json({error:'TEST FIXTURE: context unavailable'}, {status:500})) : original(input, init);
  })()`);
  browser("click", ".chat-pinned-message > p");
  wait('document.querySelector(".chat-pins-dialog [role=alert]")?.textContent.includes("context unavailable")');
  screenshot("pins-jump-error");
  evaluate("window.failContext = false");
  evaluate('document.querySelector(".chat-pinned-navigation button").focus()');
  browser("press", "Enter");
  wait(`!document.querySelector('.chat-pins-dialog') && !!document.querySelector('${row}.chat-message-jump-target')`);
  openPins();
  browser("hover", ".chat-pinned-message > p");
  browser("click", ".chat-pinned-message .chat-message-actions-trigger");
  action("Unpin message");
  wait('document.querySelector(".chat-pins-dialog")?.textContent.includes("No pinned messages.")');
  assert.equal((await history()).pinnedMessages.length, 0);
  action("Close pins");
  assert.equal(
    evaluate(`!!document.querySelector('${row} .chat-pin-marker')`),
    false,
    "unpin removes timeline attribution",
  );
  await control({ incomingReply: { rootId: root.id, text: "Pinned thread reply" } });
  const reply = (await history(root.id)).messages.at(-1);
  await control({ incomingPin: { channelId: reply.channelId, messageId: reply.id } });
  openPins();
  browser("click", ".chat-pinned-message > p");
  wait(
    `!document.querySelector('.chat-pins-dialog') && !!document.querySelector('.chat-thread-panel [data-message-id="${reply.id}"]')`,
  );
  assert.equal(
    evaluate(
      `document.querySelector('.chat-thread-panel [data-message-id="${reply.id}"] .chat-pin-author').textContent`,
    ),
    "Pinned by Alex",
  );
  console.log(
    "PASS: GIF pin survives reload; persistent gold timeline highlight at rest/on hover and self/other attribution in chat and Pins at desktop/narrow widths; live pinner updates; pinner/media/profile controls; proportional previews; body/keyboard jumps; failed jump retry; unpin removes attribution/highlight; pinned thread attribution and jump.",
  );
} finally {
  await mediaServer?.terminate();
  browser("close");
}
