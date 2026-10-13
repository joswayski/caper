// Real channel/DM composers against the disposable native-parity fixture, never live accounts.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.PASTE_TEST_WEB_URL ?? "http://localhost:5174";
const api = "http://127.0.0.1:3001";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(web).hostname));
assert.equal(
  (await fetch(`${api}/health`, { headers: { connection: "close" } }).then((response) => response.json())).fixture,
  true,
);
const artifacts = process.env.PASTE_TEST_ARTIFACTS && resolve(process.env.PASTE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...command) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "chat-paste", ...command, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (source) => browser("eval", `(() => { ${source} })()`).result;
const wait = (source) => browser("wait", "--fn", source);
const count = () => evaluate('return document.querySelectorAll(".chat-composer .chat-drafts li").length');
const value = () => evaluate('return document.querySelector("#chat-message").value');
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
}
function files(amount, input = "#chat-message", prefix = "notes") {
  return evaluate(`const data = new DataTransfer();
    data.setData('text/plain', 'file:///do-not-insert-this-path');
    for (let i = 0; i < ${amount}; i++) data.items.add(new File(['TEST FIXTURE notes ' + i], ${JSON.stringify(prefix)} + '-' + i + '.txt', {type:'text/plain'}));
    const event = new ClipboardEvent('paste', {clipboardData:data, bubbles:true, cancelable:true});
    document.querySelector(${JSON.stringify(input)}).dispatchEvent(event);
    return event.defaultPrevented;`);
}
function removeFiles() {
  while (count()) browser("click", '.chat-composer .chat-drafts li:first-child [aria-label^="Remove "]');
}
function pasteImage(input = "#chat-message", tray = ".chat-composer") {
  browser("click", input);
  evaluate(`return (async () => {
    const canvas = document.createElement('canvas'); canvas.width = 240; canvas.height = 120;
    const ctx = canvas.getContext('2d'); ctx.fillStyle = '#151719'; ctx.fillRect(0,0,240,120);
    ctx.fillStyle = '#f1ede4'; ctx.font = '18px sans-serif'; ctx.fillText('PASTED IMAGE', 36, 55);
    ctx.font = '12px sans-serif'; ctx.fillText('TEST FIXTURE', 72, 80);
    const blob = await new Promise(resolve => canvas.toBlob(resolve));
    await navigator.clipboard.write([new ClipboardItem({'image/png':blob})]);
  })();`);
  browser("press", "Control+v");
  wait(`document.querySelectorAll('${tray} .chat-drafts li').length === 1`);
}
try {
  await control({ reset: true, uploadsEnabled: true });
  browser("open", web);
  browser("cookies", "set", "caper_fixture", "owner");
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "800", "2");
    browser("open", `${web}/spaces`);
    wait('!!document.querySelector("#chat-message:not(:disabled)")');
    wait("!document.querySelector('.chat-composer [aria-label=\"Attach files\"]').disabled");
    browser("fill", "#chat-message", "Keep suffix");
    browser("click", "#chat-message");
    evaluate('document.querySelector("#chat-message").setSelectionRange(5, 5)');
    evaluate("return navigator.clipboard.writeText('plain text ')");
    browser("press", "Control+v");
    assert.equal(value(), "Keep plain text suffix", "normal paste inserts at the caret");
    assert.equal(count(), 0);
    pasteImage();
    assert.equal(value(), "Keep plain text suffix", "image paste preserves the text draft");
    assert.equal(files(1), true, "file paste must suppress its text/path representation");
    assert.equal(count(), 2);
    assert.equal(value(), "Keep plain text suffix");
    wait('!!document.querySelector(".chat-composer .chat-drafts img")');
    if (artifacts) {
      evaluate("return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
      browser("screenshot", `${artifacts}/chat-paste-${width}.png`);
    }
    // Removing an image leaves the other attachment and draft unchanged.
    browser("click", '.chat-composer .chat-drafts li:first-child [aria-label^="Remove "]');
    assert.equal(count(), 1);
    browser("press", "Control+End");
    browser("click", "#chat-message");
    browser("press", "Enter");
    wait(
      'document.querySelector("#chat-message").value === "" && !document.querySelector(".chat-composer .chat-drafts li, .chat-message-pending")',
    );
    const history = await fetch(`${api}/api/chat/channels/chan00000001/messages`, {
      headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
    }).then((response) => response.json());
    const sent = history.messages.at(-1);
    assert.equal(sent.content.text, "Keep plain text suffix");
    assert.equal(sent.content.attachments[0].name, "notes-0.txt");
    assert.equal(files(11), true);
    assert.equal(count(), 10);
    assert.ok(
      evaluate(
        'return document.querySelector(".chat-composer").textContent.includes("Only 10 more files can be attached.")',
      ),
    );
    assert.equal(files(1), true);
    assert.equal(count(), 10, "a full tray never exceeds the limit");
    assert.ok(
      evaluate(
        'return document.querySelector(".chat-composer").textContent.includes("You can attach up to 10 files.")',
      ),
    );
    removeFiles();
    console.log(
      `PASS ${width}px: native clipboard image/text, multi-file paste, text preserved, removal, send, attachment limit`,
    );
  }
  browser("set", "viewport", "1280", "800", "2");
  browser("click", ".direct-self");
  wait('document.querySelector("#chat-message")?.placeholder === "Message Fixture Owner"');
  wait("!document.querySelector('.chat-composer [aria-label=\"Attach files\"]').disabled");
  pasteImage();
  assert.equal(value(), "");
  browser("press", "Enter");
  wait('!document.querySelector(".chat-composer .chat-drafts li, .chat-message-pending")');
  const directs = await fetch(`${api}/api/dms`, {
    headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
  }).then((response) => response.json());
  const dm = directs.conversations.find((conversation) => conversation.peer.username === "fixture_owner");
  const history = await fetch(`${api}/api/chat/channels/${dm.id}/messages`, {
    headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
  }).then((response) => response.json());
  assert.equal(history.messages.at(-1).content.text, "");
  // This fixture reports every asset's kind as "file"; it retains the real upload MIME and byte size.
  assert.equal(history.messages.at(-1).content.attachments.length, 1);
  assert.match(history.messages.at(-1).content.attachments[0].contentType, /^image\//);
  assert.ok(history.messages.at(-1).content.attachments[0].size > 0);
  console.log("PASS DM: image-only clipboard send");
  // Exercise the real thread composer, not just the already-capable send client.
  const headers = { authorization: "Bearer fixture-owner-token", connection: "close" };
  const channelHistory = async () =>
    (await fetch(`${api}/api/chat/channels/chan00000001/messages`, { headers })).json();
  const roots = (await channelHistory()).messages.slice(0, 2);
  const threadFiles = () =>
    evaluate('return document.querySelectorAll(".chat-thread-composer .chat-drafts li").length');
  const channelFiles = () => evaluate('return document.querySelectorAll(".chat-panel .chat-drafts li").length');
  const openThread = (root) => {
    wait(
      '!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-panel .chat-scroller[aria-hidden=false]")',
    );
    wait(
      '!document.documentElement.dataset.browseTransition && !!document.querySelector(".stage:not([inert]) #chat-message:not(:disabled)")',
    );
    const row = `.chat-panel .chat-scroller[aria-hidden=false] [data-message-key="${root.clientMessageId}"]`;
    browser("scrollintoview", row);
    const trigger = evaluate(`return ${JSON.stringify([`${row} .chat-reply-thread`, `${row} .chat-thread-summary`])}
      .find(selector => document.querySelector(selector)?.checkVisibility({ visibilityProperty: true }));`);
    if (trigger) {
      browser("focus", trigger);
      browser("click", trigger);
    } else {
      browser("focus", `${row} .chat-message-actions-trigger`);
      browser("click", `${row} .chat-message-actions-trigger`);
      browser("find", "role", "button", "click", "--name", "Reply in thread", "--exact");
    }
    wait('!!document.querySelector("#chat-thread-reply:not(:disabled)")');
    wait("!document.querySelector('.chat-thread-composer [aria-label=\"Attach files\"]').disabled");
  };
  const closeThread = () => {
    browser("press", "Escape");
    wait('!document.querySelector(".chat-thread-panel") && !history.state?.caperThread');
  };
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "800", "2");
    browser("open", `${web}/spaces`);
    wait('!!document.querySelector("#chat-message:not(:disabled)")');
    wait("!document.querySelector('.chat-panel [aria-label=\"Attach files\"]').disabled");
    files(1, "#chat-message", "channel-only");
    openThread(roots[0]);
    pasteImage("#chat-thread-reply", ".chat-thread-composer");
    files(1, "#chat-thread-reply", "thread-only");
    assert.equal(threadFiles(), 2);
    assert.equal(channelFiles(), 1);
    if (artifacts) browser("screenshot", `${artifacts}/thread-attachments-${width}.png`);
    closeThread();
    openThread(roots[1]);
    assert.equal(threadFiles(), 0, "another thread has its own tray");
    files(1, "#chat-thread-reply", "other-thread");
    closeThread();
    // A conversation switch must also retain each thread's in-progress files.
    if (width === 390) {
      browser("click", ".navigation-toggle");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.documentElement.dataset.browseTransition',
      );
    }
    browser("click", ".direct-self");
    wait('document.querySelector("#chat-message")?.placeholder === "Message Fixture Owner"');
    if (width === 390) {
      wait("!document.documentElement.dataset.browseTransition");
      browser("click", ".navigation-toggle");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.documentElement.dataset.browseTransition',
      );
    }
    browser("find", "role", "button", "click", "--name", "general", "--exact");
    wait('document.querySelector("#chat-message")?.placeholder === "Message #general"');
    openThread(roots[0]);
    assert.equal(threadFiles(), 2);
    assert.equal(channelFiles(), 1);
    browser("click", '.chat-thread-send-row [aria-label="Send reply"]');
    wait('!document.querySelector(".chat-thread-panel .chat-message-pending, .chat-thread-composer .chat-drafts li")');
    const page = await fetch(`${api}/api/chat/channels/chan00000001/messages/${roots[0].id}/thread`, { headers }).then(
      (response) => response.json(),
    );
    const reply = page.messages.at(-1);
    assert.equal(reply.content.text, "", "attachments alone enable Send reply");
    assert.equal(reply.threadRootId, roots[0].id);
    assert.equal(reply.broadcast, false);
    assert.equal(reply.content.attachments.length, 2);
    assert.match(reply.content.attachments[0].contentType, /^image\//);
    assert.equal(reply.content.attachments[1].name, "thread-only-0.txt");
    assert.equal(
      (await channelHistory()).messages.some((message) => message.id === reply.id),
      false,
    );
    assert.equal(channelFiles(), 1, "sending a reply leaves channel files untouched");
    closeThread();
    openThread(roots[1]);
    assert.equal(threadFiles(), 1, "sending in the first thread leaves the second draft untouched");
    browser("check", ".chat-thread-send-row input");
    browser("click", '.chat-thread-send-row [aria-label="Send reply"]');
    wait('!document.querySelector(".chat-thread-panel .chat-message-pending, .chat-thread-composer .chat-drafts li")');
    const broadcast = (await channelHistory()).messages.at(-1);
    assert.equal(broadcast.threadRootId, roots[1].id);
    assert.equal(broadcast.broadcast, true);
    assert.equal(broadcast.content.attachments[0].name, "other-thread-0.txt");
    closeThread();
    removeFiles();
    console.log(
      `PASS ${width}px threads: image/file-only sends, broadcast, isolated and retained channel/thread drafts`,
    );
  }
  await control({ uploadsEnabled: false });
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  wait(
    'document.querySelector(\'.chat-composer [aria-label="Attach files"]\')?.title === "File uploads are unavailable"',
  );
  assert.equal(files(1), false);
  assert.equal(count(), 0);
  browser("fill", "#chat-message", "");
  browser("click", "#chat-message");
  evaluate("return navigator.clipboard.writeText('text still works')");
  browser("press", "Control+v");
  assert.equal(value(), "text still works");
  console.log("PASS unavailable uploads: files ignored, ordinary text paste preserved");
} finally {
  browser("close");
  await control({ reset: true });
}
