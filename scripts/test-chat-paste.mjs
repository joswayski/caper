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
    await control({ holdSends: true });
    browser("press", "Control+End");
    browser("click", "#chat-message");
    browser("press", "Enter");
    wait('!!document.querySelector(".chat-message-pending .chat-attachments")');
    const pendingLayout = evaluate(`const row = document.querySelector('.chat-message-pending');
      const box = row.querySelector('.chat-attachments').getBoundingClientRect();
      return { key: row.dataset.messageKey, x: box.x, width: box.width };`);
    await control({ holdSends: false });
    wait(
      'document.querySelector("#chat-message").value === "" && !document.querySelector(".chat-composer .chat-drafts li, .chat-message-pending")',
    );
    const sentLayout = evaluate(`const row = document.querySelector('[data-message-key="${pendingLayout.key}"]');
      const box = row.querySelector('.chat-attachments').getBoundingClientRect();
      return { key: row.dataset.messageKey, x: box.x, width: box.width };`);
    assert.deepEqual(sentLayout, pendingLayout, "confirmation never narrows or shifts attachment space");
    const history = await fetch(`${api}/api/chat/channels/chan00000001/messages`, {
      headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
    }).then((response) => response.json());
    const sent = history.messages.at(-1);
    assert.equal(sent.content.text, "Keep plain text suffix");
    assert.equal(sent.content.attachments[0].name, "notes-0.txt");

    // A ready file must not be delivered on its own when another upload fails.
    browser("fill", "#chat-message", "Send both files together");
    files(1, "#chat-message", "ready");
    wait('!document.querySelector(".chat-composer .chat-drafts progress")');
    await control({ failure: { path: "/api/assets", method: "POST", status: 503, persistent: true } });
    files(1, "#chat-message", "retry");
    wait('!!document.querySelector(".chat-composer .chat-draft-notice[data-error]")');
    const warning = evaluate('return document.querySelector(".chat-composer .chat-draft-notice").textContent');
    assert.equal(warning, "1 file failed to upload. Send will retry it, or you can remove it.");
    if (artifacts) browser("screenshot", `${artifacts}/upload-retry-failed-${width}.png`);
    const readMessages = () =>
      fetch(`${api}/api/chat/channels/chan00000001/messages`, {
        headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
      }).then((response) => response.json());
    const before = (await readMessages()).messages.length;
    const submit = () => {
      if (width === 1280) {
        browser("click", "#chat-message");
        browser("press", "Enter");
      } else browser("click", '.chat-composer [aria-label="Send message"]');
    };
    for (const succeeds of [false, true]) {
      await control({ holdUploads: true, clearFailures: succeeds });
      submit();
      wait(
        '!!document.querySelector(".chat-composer .chat-drafts progress") && !document.querySelector(".chat-composer .chat-draft-notice[data-error]")',
      );
      assert.equal(
        evaluate('return document.querySelector(".chat-composer .chat-draft-notice").textContent'),
        "Uploading 1 file… Your message won’t send until every file is ready.",
      );
      assert.equal((await readMessages()).messages.length, before, "uploading blocks delivery of the complete message");
      if (artifacts && !succeeds) browser("screenshot", `${artifacts}/upload-retry-uploading-${width}.png`);
      // A second press must share the same attempt, not submit another message.
      submit();
      await control({ holdUploads: false });
      if (!succeeds) {
        wait('!!document.querySelector(".chat-composer .chat-draft-notice[data-error]")');
        assert.equal(value(), "Send both files together");
        assert.equal(count(), 2);
        assert.equal(
          (await readMessages()).messages.length,
          before,
          "a repeated failure never sends just the ready file",
        );
      } else {
        wait('!document.querySelector(".chat-composer .chat-drafts li, .chat-message-pending")');
        const after = (await readMessages()).messages;
        assert.equal(
          after.length,
          before + 1,
          "one Send continues automatically after retry, without duplicate delivery",
        );
        assert.deepEqual(
          after.at(-1).content.attachments.map((file) => file.name),
          ["ready-0.txt", "retry-0.txt"],
        );
        assert.equal(value(), "");
      }
    }
    console.log(
      `PASS ${width}px upload retries: visible progress, repeated failure retains draft, no subset/duplicate send, automatic completion`,
    );
    assert.equal(files(11), true);
    assert.equal(count(), 10);
    const grid = evaluate(`const tray = document.querySelector('.chat-composer .chat-drafts');
      return { width: tray.clientWidth, scrollWidth: tray.scrollWidth, text: tray.innerText,
        tiles: [...tray.children].map(tile => {
          const { x, y, width, height } = tile.getBoundingClientRect();
          return { x, y, width, height };
        }) };`);
    const firstRow = grid.tiles.filter((tile) => Math.abs(tile.y - grid.tiles[0].y) < 1);
    assert.equal(
      firstRow.length,
      width === 1280 ? Math.floor((grid.width + 8) / 112) : 3,
      "desktop fills the composer width; mobile keeps compact capped rows",
    );
    if (width === 1280) assert.ok(firstRow.length > 4, "desktop is not capped at four columns");
    assert.ok(grid.tiles[firstRow.length].y > grid.tiles[0].y, "extra files wrap vertically");
    assert.ok(grid.tiles.every((tile) => Math.abs(tile.width - tile.height) < 1 && tile.width <= 104));
    assert.equal(grid.scrollWidth, grid.width, "the tray never scrolls sideways");
    assert.ok(!grid.text.includes("notes-"), "filenames are not visible in the upload grid");
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
  // Recent roots stay mounted in the virtualized viewport after the upload scenarios above.
  const roots = (await channelHistory()).messages.slice(-2);
  const threadFiles = () =>
    evaluate('return document.querySelectorAll(".chat-thread-composer .chat-drafts li").length');
  const channelFiles = () => evaluate('return document.querySelectorAll(".chat-panel .chat-drafts li").length');
  const openThread = (root) => {
    wait(
      '!document.querySelector(".chat-initial-messages") && !!document.querySelector(".chat-panel .chat-scroller[aria-hidden=false]")',
    );
    wait(
      '!document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide") && !!document.querySelector(".stage:not([inert]) #chat-message:not(:disabled)")',
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
    if (width === 1280) {
      files(9, "#chat-message", "split-pane");
      const split = evaluate(`const tray = document.querySelector('.chat-panel .chat-drafts');
        return { height: tray.clientHeight, scrollHeight: tray.scrollHeight,
          width: tray.clientWidth, scrollWidth: tray.scrollWidth,
          editorBottom: document.querySelector('#chat-message').getBoundingClientRect().bottom,
          viewportHeight: innerHeight };`);
      assert.ok(split.editorBottom <= split.viewportHeight, "a full narrow tray never pushes the editor offscreen");
      assert.ok(split.scrollHeight > split.height, "tall desktop trays scroll vertically within the composer");
      assert.equal(split.scrollWidth, split.width, "split panes never scroll sideways");
      while (channelFiles() > 1) browser("click", '.chat-panel .chat-drafts li:last-child [aria-label^="Remove "]');
    }
    pasteImage("#chat-thread-reply", ".chat-thread-composer");
    wait('!document.querySelector(".chat-thread-composer .chat-drafts progress")');
    await control({ failure: { path: "/api/assets", method: "POST", status: 503, persistent: true } });
    files(1, "#chat-thread-reply", "thread-only");
    wait('!!document.querySelector(".chat-thread-composer .chat-draft-notice[data-error]")');
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
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide")',
      );
    }
    browser("click", ".direct-self");
    wait('document.querySelector("#chat-message")?.placeholder === "Message Fixture Owner"');
    if (width === 390) {
      wait('!document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide")');
      browser("click", ".navigation-toggle");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide")',
      );
    }
    browser("find", "role", "button", "click", "--name", "general", "--exact");
    wait('document.querySelector("#chat-message")?.placeholder === "Message #general"');
    openThread(roots[0]);
    assert.equal(threadFiles(), 2);
    assert.equal(channelFiles(), 1);
    browser("click", '.chat-thread-send-row [aria-label="Send reply"]');
    wait('!!document.querySelector(".chat-thread-composer .chat-draft-notice[data-error]")');
    assert.equal(threadFiles(), 2, "a failed Send retains the entire thread selection");
    await control({ clearFailures: true, holdUploads: true });
    browser("click", '.chat-thread-send-row [aria-label="Send reply"]');
    wait('!!document.querySelector(".chat-thread-composer .chat-drafts progress")');
    if (artifacts) browser("screenshot", `${artifacts}/upload-retry-thread-${width}.png`);
    await control({ holdUploads: false });
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
