// Real profile UI, disposable loopback fixture. Never run against live accounts.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const web = process.env.PROFILE_TEST_WEB_URL ?? "http://localhost:5174";
const api = "http://127.0.0.1:3001";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(web).hostname));
assert.equal(
  (await fetch(`${api}/health`, { headers: { connection: "close" } }).then((response) => response.json())).fixture,
  true,
);
const artifacts = process.env.PROFILE_TEST_ARTIFACTS && resolve(process.env.PROFILE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), "caper-profile-"));
const file = join(scratch, "review-notes.txt");
writeFileSync(file, "TEST FIXTURE — profile attachment\n");
const session = "profile-composer";
function browser(...command) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, ...command, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (source) => browser("eval", `(() => { ${source} })()`).result;
const wait = (source) => browser("wait", "--fn", source);
const value = () => evaluate('return document.querySelector("#profile-message").value');
const open = (name) => {
  const selector = `.chat-author-profile[aria-label="Open profile for ${name}"]`;
  wait(`(() => {
    const button = document.querySelector(${JSON.stringify(selector)});
    if (!button) return false;
    const box = button.getBoundingClientRect();
    return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2)?.closest(${JSON.stringify(selector)}) === button;
  })()`);
  try {
    browser("click", selector);
  } catch {
    // History can still be settling (e.g. scrolling to a new request); retry once.
    evaluate("return new Promise(r => setTimeout(r, 500))");
    browser("click", selector);
  }
  wait('!!document.querySelector("#profile-message")');
};
const close = () => {
  browser("click", '.chat-mention-card [aria-label="Close profile"]');
  wait('!document.querySelector(".chat-mention-card")');
};
const capture = (name) => {
  if (!artifacts) return;
  evaluate("return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  browser("screenshot", ".chat-mention-card", `${artifacts}/${name}.png`);
};
const control = async (body) => {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
};
const directs = async () =>
  (
    await fetch(`${api}/api/dms`, {
      headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
    }).then((response) => response.json())
  ).conversations;
const history = (channel) =>
  fetch(`${api}/api/chat/channels/${channel}/messages`, {
    headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
  }).then((response) => response.json());

try {
  await control({ reset: true, uploadsEnabled: true });
  browser("open", web);
  browser("cookies", "set", "caper_fixture", "owner");
  browser("open", `${web}/spaces`);
  browser("set", "viewport", "1280", "800", "2");
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  browser("fill", "#chat-message", "Existing channel draft");
  evaluate(`const realFetch = window.fetch.bind(window); window.assetReservations = [];
    window.fetch = (input, options) => {
      if (options?.method === 'POST' && String(input) === '/api/assets') window.assetReservations.push(JSON.parse(options.body));
      return realFetch(input, options);
    };`);
  wait("!document.querySelector('.chat-composer [aria-label=\"Attach files\"]').disabled");
  browser("upload", '.chat-composer input[type="file"]', file);
  wait('document.querySelector(".chat-drafts li small")?.textContent === "36 B"');
  const avatar = '.chat-avatar.chat-author-profile[aria-label="Open profile for Maya"]';
  browser("hover", avatar);
  evaluate("return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('${avatar}')).outlineStyle`), "none");
  open("Maya");
  assert.equal(evaluate(`return getComputedStyle(document.querySelector('${avatar}')).outlineStyle`), "none");
  assert.equal(evaluate("return document.activeElement.id"), "profile-message");
  assert.equal(evaluate('return document.querySelector(".chat-mention-card").textContent.includes("Block")'), false);
  assert.equal(evaluate('return !!document.querySelector(".chat-profile-composer .lucide-paperclip")'), true);
  capture("profile-desktop-empty");
  browser("click", '.chat-mention-card [aria-label="More profile actions"]');
  wait('!!document.querySelector(".chat-profile-menu")');
  assert.equal(evaluate('return document.querySelector(".chat-profile-menu button").textContent'), "Block");
  if (artifacts) browser("screenshot", `${artifacts}/profile-desktop-menu.png`);
  browser("press", "Escape");
  wait('!document.querySelector(".chat-profile-menu")');
  assert.equal(value(), "", "Escape closes only the child menu");
  browser("click", '.chat-mention-card [aria-label="More profile actions"]');
  wait('!!document.querySelector(".chat-profile-menu")');
  browser("click", ".chat-profile-menu button");
  wait('document.querySelector("dialog[open] h2")?.textContent === "Block Maya?"');
  assert.equal(evaluate('return !!document.querySelector(".chat-mention-card")'), false);
  browser("click", "dialog[open] [data-initial-focus]");
  open("Maya");
  browser("fill", "#profile-message", "Hello suffix");
  evaluate('document.querySelector("#profile-message").setSelectionRange(6, 6)');
  // Cancelling a chooser keeps both the draft and the selected caret.
  evaluate('document.querySelector(\'.chat-profile-composer input[type="file"]\').dispatchEvent(new Event("cancel"))');
  wait('document.activeElement.id === "profile-message"');
  assert.equal(value(), "Hello suffix");
  assert.equal(evaluate('return document.querySelector("#profile-message").selectionStart'), 6);
  browser("click", '.chat-profile-composer [aria-label="Insert emoji"]');
  wait('!!document.querySelector(".chat-emoji-grid")');
  capture("profile-desktop-emoji");
  browser("click", '.chat-reaction-picker .chat-emoji-grid button[data-unified="1f600"]');
  assert.equal(value(), "Hello 😀suffix", "Emoji goes at the saved caret, not at the end");
  assert.equal(evaluate("return document.activeElement.id"), "profile-message");
  // Browser-supported file upload, then a file-bearing paste event.
  wait(`!document.querySelector('.chat-profile-composer [aria-label="Attach files"]').disabled`);
  browser("upload", '.chat-profile-composer input[type="file"]', file);
  evaluate(`const transfer = new DataTransfer();
    transfer.items.add(new File(['TEST FIXTURE pasted bytes'], 'pasted.txt', {type:'text/plain'}));
    document.querySelector('#profile-message').dispatchEvent(new ClipboardEvent('paste', {clipboardData:transfer, bubbles:true, cancelable:true}));`);
  assert.equal(evaluate('return document.querySelectorAll(".chat-profile-composer .chat-drafts li").length'), 2);
  assert.equal((await directs()).length, 0, "Typing and staging files must not create a DM");
  capture("profile-desktop-files");
  browser("mouse", "move", "900", "100");
  browser("mouse", "down", "left");
  browser("mouse", "up", "left");
  wait('!document.querySelector(".chat-mention-card")');
  open("Alex");
  assert.equal(value(), "", "Each person gets their own draft");
  close();
  open("Maya");
  assert.equal(value(), "Hello 😀suffix");
  assert.equal(evaluate('return document.querySelectorAll(".chat-profile-composer .chat-drafts li").length'), 2);
  // Lose the HTTP acknowledgement after the fixture has accepted the message.
  evaluate(`const realFetch = window.fetch.bind(window); window.profileCommands = [];
    window.dropProfileAck = true;
    window.fetch = async (input, options) => {
      const send = options?.method === 'POST' && new RegExp('^/api/chat/channels/[^/]+/messages$').test(String(input));
      if (send) window.profileCommands.push(JSON.parse(options.body));
      const response = await realFetch(input, options);
      if (send && window.dropProfileAck) { window.dropProfileAck = false; throw new TypeError('TEST FIXTURE lost acknowledgement'); }
      return response;
    };`);
  await control({ holdSends: true });
  browser("click", '.chat-profile-composer button[type="submit"]');
  wait('document.querySelector("#profile-message")?.disabled');
  browser("mouse", "move", "900", "100");
  browser("mouse", "down", "left");
  browser("mouse", "up", "left");
  assert.equal(value(), "Hello 😀suffix", "An active send still prevents dismissal");
  await control({ holdSends: false });
  wait('document.querySelector(".chat-profile-composer button[type=submit]")?.ariaLabel === "Retry send"');
  assert.equal(value(), "Hello 😀suffix");
  assert.equal(evaluate('return document.querySelector("#profile-message").disabled'), true);
  assert.equal(evaluate('return document.querySelector("#chat-message").value'), "Existing channel draft");
  capture("profile-desktop-unconfirmed");
  const dm = (await directs()).find((conversation) => conversation.peer.username === "maya");
  assert.equal((await history(dm.id)).messages.length, 1, "Message persisted despite the lost acknowledgement");
  close();
  open("Maya");
  assert.ok(
    evaluate('return document.querySelector(".chat-profile-composer button[type=submit]").ariaLabel') === "Retry send",
  );
  browser("click", '.chat-profile-composer button[type="submit"]');
  wait(
    '!document.querySelector(".chat-mention-card") && document.querySelector(".chat-heading")?.textContent.includes("Maya")',
  );
  const commands = evaluate("return window.profileCommands");
  assert.deepEqual(commands[1], commands[0], "Retry reuses the identical message ID, text and attachment IDs");
  const sent = (await history(dm.id)).messages;
  assert.equal(sent.length, 1, "Retry must not duplicate the DM");
  assert.equal(sent[0].content.text, "Hello 😀suffix");
  assert.deepEqual(
    sent[0].content.attachments.map((item) => item.name),
    ["review-notes.txt", "pasted.txt"],
  );
  assert.equal((await history("chan00000001")).messages.length, 4, "Nothing was sent to the active space channel");
  // Return to channel and exercise a rejected send plus narrow-sheet layout.
  browser("click", ".channel-select");
  wait(`!!document.querySelector('.chat-author-profile[aria-label="Open profile for Maya"]')`);
  assert.equal(evaluate('return document.querySelector("#chat-message").value'), "Existing channel draft");
  // Like any conversation switch, the channel keeps its text and files, and
  // returning reuses the uploaded file instead of uploading it again.
  wait('document.querySelector(".chat-composer .chat-drafts li strong")?.textContent === "review-notes.txt"');
  assert.equal(evaluate('return window.assetReservations.filter(item => item.channelId === "chan00000001").length'), 1);
  open("Maya");
  assert.equal(value(), "", "Confirmed send clears only the profile draft");
  browser("fill", "#profile-message", "Rejected draft");
  await control({
    failure: {
      path: `/api/chat/channels/${dm.id}/messages`,
      method: "POST",
      status: 400,
      error: "TEST FIXTURE refused message",
    },
  });
  browser("click", '.chat-profile-composer button[type="submit"]');
  wait('document.querySelector(".chat-profile-composer [role=alert]")?.textContent.includes("refused")');
  assert.equal(value(), "Rejected draft");
  assert.equal(evaluate('return document.querySelector("#profile-message").disabled'), false);
  close();
  browser("set", "viewport", "390", "844", "2");
  open("Maya");
  assert.equal(value(), "Rejected draft");
  browser("fill", "#profile-message", "");
  capture("profile-narrow-empty");
  browser("fill", "#profile-message", "Rejected draft");
  // Each card checks upload settings when it opens; files wait for that.
  wait(`!document.querySelector('.chat-profile-composer [aria-label="Attach files"]').disabled`);
  browser("upload", '.chat-profile-composer input[type="file"]', file);
  wait('document.querySelectorAll(".chat-profile-composer .chat-drafts li").length === 1');
  capture("profile-narrow-files");
  assert.equal(evaluate('return !!document.querySelector(".chat-mention-card-drawer")'), true);
  // The sheet slides in; wait for it to settle inside the viewport.
  wait(
    '(() => { const card=document.querySelector(".chat-mention-card").getBoundingClientRect(); return card.left>=0 && card.right<=innerWidth && card.bottom<=innerHeight + 0.5; })()',
  );
  assert.equal(value(), "Rejected draft");
  browser("click", '.chat-mention-card [aria-label="More profile actions"]');
  wait('!!document.querySelector(".chat-profile-menu")');
  if (artifacts) browser("screenshot", `${artifacts}/profile-narrow-menu.png`);
  browser("press", "Escape");
  wait('!document.querySelector(".chat-profile-menu")');
  // A backdrop press closes a drafted sheet; reopening restores text and files.
  browser("mouse", "move", "195", "80");
  browser("mouse", "down", "left");
  browser("mouse", "up", "left");
  wait('!document.querySelector(".chat-mention-card")');
  open("Maya");
  assert.equal(value(), "Rejected draft");
  assert.equal(evaluate('return document.querySelectorAll(".chat-profile-composer .chat-drafts li").length'), 1);
  browser("click", '.chat-profile-composer [aria-label="Insert emoji"]');
  wait('!!document.querySelector(".chat-emoji-grid")');
  // The eager grid mounts before the drawer's entrance animation finishes.
  wait(
    '(() => { const picker=document.querySelector(".chat-emoji-picker").getBoundingClientRect(); return picker.left >= 0 && picker.right <= innerWidth && picker.bottom <= innerHeight; })()',
  );
  capture("profile-narrow-emoji");
  assert.ok(
    evaluate(
      'const picker=document.querySelector(".chat-emoji-picker").getBoundingClientRect(); return picker.left >= 0 && picker.right <= innerWidth && picker.bottom <= innerHeight',
    ),
  );
  browser("press", "Escape");
  wait('!document.querySelector(".chat-emoji-picker")');
  assert.equal(value(), "Rejected draft", "Escape closes emoji first without closing the profile");
  browser("focus", "#profile-message");
  browser("press", "Control+a");
  browser("press", "Backspace");
  assert.equal(value(), "");
  browser("click", '.chat-profile-composer button[type="submit"]');
  wait('!document.querySelector(".chat-mention-card")');
  const fileOnly = (await history(dm.id)).messages;
  assert.equal(fileOnly.length, 2);
  assert.equal(fileOnly[1].content.text, "");
  assert.equal(
    fileOnly[1].content.attachments[0].name,
    "review-notes.txt",
    "File-only sends work without placeholder text",
  );
  // Incoming requests remain previews until the account explicitly accepts.
  await control({ messageRequest: { text: "TEST FIXTURE — incoming request" } });
  browser("set", "viewport", "1280", "800", "2");
  browser("open", `${web}/spaces`);
  // Wait for restored conversation loading, not just the sidebar's first paint.
  wait('!!document.querySelector("#chat-message:not(:disabled)") && !!document.querySelector(".direct-requests")');
  browser("click", ".direct-requests");
  browser("click", "#direct-request-list .direct-select");
  wait('!!document.querySelector(".direct-request-bar")');
  open("Jordan");
  browser("fill", "#profile-message", "Keep my request reply");
  browser("click", '.chat-profile-composer button[type="submit"]');
  wait('document.querySelector(".chat-profile-composer [role=alert]")?.textContent.includes("accept")');
  assert.equal(value(), "Keep my request reply");
  assert.equal((await directs()).find((item) => item.peer.username === "jordan").status, "incoming");
  browser("click", '.chat-mention-card [aria-label="Open conversation"]');
  wait('!document.querySelector(".chat-mention-card")');
  assert.equal(
    (await directs()).find((item) => item.peer.username === "jordan").status,
    "incoming",
    "Open conversation must not silently accept a request",
  );
  console.log(
    "PASS: secondary Block menu/confirmation, profile text/emoji/files and file-only sends, caret/chooser cancellation, outside dismissal with retained drafts, active-send protection, person-keyed drafts, uncertain idempotent retry, rejection, retained channel text/files, request acceptance isolation, desktop/narrow layouts.",
  );
} catch (error) {
  // Preserve the original failure even if the browser itself stopped responding.
  try {
    console.error(browser("snapshot", "-i"));
  } catch {
    console.error("Browser diagnostics unavailable.");
  }
  throw error;
} finally {
  browser("close");
  rmSync(scratch, { recursive: true, force: true });
  await control({ holdSends: false, clearFailures: true });
}
