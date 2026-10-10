// Uses only the explicitly labelled, loopback-only native parity fixture.
// Start native-parity-fixture.mjs and Vite first. Never target a real account/API.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.DM_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.DM_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api])
  assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname), "Use a disposable loopback fixture");
const artifacts = process.env.DM_TEST_ARTIFACTS && resolve(process.env.DM_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const args = ["--session", "direct-message-test"];
function browser(...command) {
  const result = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60_000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (source) => browser("eval", source).result;
const wait = (expression) => browser("wait", "--fn", expression);
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  browser("screenshot", `${artifacts}/${name}.png`);
};
const checkSidebarAlignment = () => {
  const rows = evaluate(`(() => {
    const selectors = ['.direct-section-title', '.direct-self', '.direct-action'];
    if (innerWidth > 760) selectors.push('.account-profile');
    return selectors.map(selector => {
      const row = document.querySelector(selector), icon = row.firstElementChild;
      const label = [...row.childNodes].find(node => node.nodeType === 3 && node.textContent.trim()) ?? icon.nextSibling;
      const range = document.createRange();
      range.selectNodeContents(label);
      const box = row.getBoundingClientRect(), iconBox = icon.getBoundingClientRect();
      return { selector, iconX: iconBox.left + iconBox.width / 2, labelX: range.getBoundingClientRect().left,
        iconOffsetY: iconBox.top + iconBox.height / 2 - (box.top + box.height / 2) };
    });
  })()`);
  for (const row of rows) {
    assert.ok(Math.abs(row.iconX - rows[0].iconX) < 0.5, `${row.selector} shares the sidebar icon/avatar center`);
    assert.ok(Math.abs(row.labelX - rows[0].labelX) < 0.5, `${row.selector} shares the sidebar label column`);
    assert.ok(Math.abs(row.iconOffsetY) < 0.5, `${row.selector} vertically centers its icon/avatar`);
  }
  const dock = evaluate(`(() => {
    const account = document.querySelector('.call-account').getBoundingClientRect();
    const avatar = document.querySelector('.direct-self .direct-avatar').getBoundingClientRect();
    const artwork = document.querySelector('.direct-self .direct-avatar > span').getBoundingClientRect();
    const panel = document.querySelector('.voice-panel'), box = panel.getBoundingClientRect();
    const style = getComputedStyle(panel), divider = getComputedStyle(panel, '::before');
    return {
      artworkEdges: [artwork.left - avatar.left, artwork.top - avatar.top, artwork.right - avatar.right, artwork.bottom - avatar.bottom],
      controlOffsets: [...document.querySelectorAll('.call-account :is(.account-avatar, .account-name, .voice-icon-button, .call-settings-trigger, .voice-icon-button > svg, .call-settings-trigger > svg)')].map(node => {
        const rect = node.getBoundingClientRect();
        return rect.top + rect.height / 2 - (account.top + account.height / 2);
      }),
      dividerGap: -parseFloat(style.borderTopWidth) - parseFloat(divider.top),
      bottomGap: panel.parentElement.getBoundingClientRect().bottom - box.bottom,
    };
  })()`);
  assert.ok(
    dock.artworkEdges.every((offset) => Math.abs(offset) < 0.5),
    "Actual self-avatar artwork fills its container without shifting or clipping",
  );
  assert.ok(
    dock.controlOffsets.every((offset) => Math.abs(offset) < 0.5),
    "Profile, microphone, headphones, dropdowns, settings and their glyphs share the dock vertical center",
  );
  if (evaluate("innerWidth > 760")) {
    assert.equal(dock.dividerGap, 12, "Dock starts 12px below the divider, not flush against it");
    assert.equal(dock.bottomGap, 12, "Dock keeps the same 12px inset below it");
  }
};
async function control(body) {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
}
async function conversations() {
  return (
    await fetch(`${api}/api/dms`, { headers: { authorization: "Bearer fixture-owner-token", connection: "close" } })
  ).json();
}
try {
  assert.equal((await (await fetch(`${api}/health`, { headers: { connection: "close" } })).json()).fixture, true);
  await control({ reset: true });
  browser("open", "about:blank");
  browser("set", "viewport", "1440", "900", "2");
  // The phone Browse slide is covered by test-desktop-navigation; settle it here.
  browser("set", "media", "reduced-motion");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector(".direct-section") && !!document.querySelector(".chat-composer textarea")');
  const initialReads = evaluate(`performance.getEntriesByType('resource')
    .filter(entry => new URL(entry.name).pathname === '/api/dms').map(entry => entry.startTime)`);
  assert.equal(
    initialReads.filter((start) => start < initialReads[0] + 10_000).length,
    1,
    "Initialization owns one DM read; the 15-second poll must not duplicate it on mount",
  );
  assert.equal(evaluate('document.querySelector(".chat-heading").textContent.includes("Leave channel")'), false);
  browser("click", '[aria-label="Manage general"]');
  assert.ok(
    evaluate(
      'document.querySelector(".channel-menu[open]").textContent.includes("Channel settings") && document.querySelector(".channel-menu[open]").textContent.includes("Leave channel")',
    ),
    "Owners retain settings and can leave through the same menu",
  );
  browser("press", "Escape");
  assert.equal(evaluate('document.querySelector(".direct-self small").textContent'), "you");
  assert.equal(
    evaluate('document.querySelector(".direct-section").textContent.includes("No direct messages yet")'),
    false,
  );
  assert.equal((await conversations()).conversations.length, 0, "Showing the self row must not create a conversation");
  const plusOpacity = () =>
    evaluate('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity');
  if (evaluate('matchMedia("(hover: hover)").matches')) {
    browser("mouse", "move", "700", "400");
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "0"');
    browser("hover", ".direct-section-title");
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "1"');
    browser("click", '.direct-section [aria-label="New direct message"]');
    wait('!!document.querySelector(".space-dialog[open]")');
    browser("click", '[aria-label="Close New direct message"]');
    browser("mouse", "move", "700", "400");
    evaluate(`(() => { const buttons = [...document.querySelectorAll('button:not(:disabled), a[href], summary, textarea:not(:disabled)')].filter(node => node.getClientRects().length);
      const plus = document.querySelector('.direct-section [aria-label="New direct message"]');
      buttons[buttons.indexOf(plus) - 1].focus(); })()`);
    browser("press", "Tab");
    assert.equal(
      evaluate('document.activeElement?.getAttribute("aria-label")'),
      "New direct message",
      "Hidden plus stays in the keyboard focus order",
    );
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "1"');
    browser("press", "Enter");
    wait('!!document.querySelector(".space-dialog[open]")');
    browser("click", '[aria-label="Close New direct message"]');
    evaluate("document.activeElement.blur()");
    wait('getComputedStyle(document.querySelector(".direct-section .channel-section-actions")).opacity === "0"');
    assert.equal(plusOpacity(), "0");
  } else {
    assert.equal(plusOpacity(), "1", "Non-hover clients keep the DM action visible");
  }
  assert.ok(
    evaluate(`(() => { const panel = document.querySelector('.people-panel').getBoundingClientRect();
    return ['.channel-navigation > header', '.direct-section'].every(selector => {
      const node = document.querySelector(selector), r = node.getBoundingClientRect();
      const s = getComputedStyle(node, selector.includes('header') ? '::after' : '::before');
      return Math.abs(r.left + parseFloat(s.left) - panel.left) < 1 && Math.abs(r.right - parseFloat(s.right) - (panel.right - 1)) < 1;
    }); })()`),
    "Sidebar dividers must span the panel, not just its padded contents",
  );
  checkSidebarAlignment();
  browser("focus", '[aria-label="Channel sidebar width"]');
  for (const [key, width] of [
    ["Home", 220],
    ["End", 440],
  ]) {
    browser("press", key);
    wait(`document.querySelector('[aria-label="Channel sidebar width"]').getAttribute('aria-valuenow') === '${width}'`);
    checkSidebarAlignment();
  }
  browser("dblclick", '[aria-label="Channel sidebar width"]');
  wait('document.querySelector("[aria-label=\\"Channel sidebar width\\"]").getAttribute("aria-valuenow") === "260"');
  screenshot("dm-empty-list");
  for (const width of [1440, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    if (width === 390) browser("click", ".navigation-toggle");
    checkSidebarAlignment();
    assert.deepEqual(
      evaluate(`(() => {
        const navigation = document.querySelector('.channel-navigation');
        const style = getComputedStyle(navigation);
        return [style.overflowY, style.scrollbarWidth, getComputedStyle(navigation, '::-webkit-scrollbar').display];
      })()`),
      ["auto", "none", "none"],
      `Navigation stays scrollable without native scrollbar chrome at ${width}px`,
    );
    console.log(
      `PASS: sidebar icon/avatar centers, label columns, and vertical centering at ${width}px (220/260/440px desktop sidebar widths)`,
    );
    browser("click", '[aria-label="New direct message"]');
    wait('!!document.querySelector(".space-dialog[open]")');
    assert.equal(
      evaluate('document.querySelector(".space-dialog > header p").textContent'),
      "Enter an exact username. Conversations stay private across all your spaces.",
    );
    browser("fill", ".space-dialog input", "fixture_alex");
    const inside = evaluate(
      '(() => { const r = document.querySelector(".space-dialog").getBoundingClientRect(); return [r.left + 8, r.top + 8]; })()',
    );
    browser("mouse", "move", ...inside.map(String));
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
    assert.equal(
      evaluate('!!document.querySelector(".space-dialog[open]")'),
      true,
      "Clicking dialog padding keeps it open",
    );
    browser("mouse", "down", "left");
    browser("mouse", "move", "2", "2");
    browser("mouse", "up", "left");
    assert.equal(
      evaluate('!!document.querySelector(".space-dialog[open]")'),
      true,
      "A drag starting inside the dialog cannot dismiss it",
    );
    assert.equal(evaluate('document.querySelector(".space-dialog input").value'), "fixture_alex");
    screenshot(`dm-start-dialog-${width}`);
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
    wait('!document.querySelector(".space-dialog[open]")');
    assert.equal(
      evaluate('document.activeElement?.getAttribute("aria-label")'),
      "New direct message",
      "Backdrop dismissal restores focus to its opener",
    );
    assert.equal((await conversations()).conversations.length, 0, "Dismissing does not create a conversation");
  }
  browser("set", "viewport", "1440", "900", "2");
  console.log(
    "PASS: shortened direct-message copy; outside click dismisses, inside clicks/drags stay open, focus returns at desktop/narrow widths",
  );
  browser("click", ".direct-action");
  wait('document.querySelector(".space-dialog h2")?.textContent === "Manage space"');
  browser("fill", ".member-add input", "sam");
  browser("click", '.member-add button[type="submit"]');
  wait('document.querySelector(".member-manager")?.textContent.includes("@sam · Invited")');
  screenshot("dm-invite-people");
  browser("click", '[aria-label="Close Manage space"]');
  browser("click", '[aria-label="New direct message"]');
  wait('!!document.querySelector(".space-dialog[open]")');
  browser("fill", ".space-dialog input", "missing_account");
  browser("click", '.space-dialog button[type="submit"]');
  wait('!!document.querySelector(".space-form-error")');
  assert.match(evaluate('document.querySelector(".space-form-error").textContent'), /not found/i);
  screenshot("dm-start-error");
  browser("fill", ".space-dialog input", "fixture_alex");
  browser("click", '.space-dialog button[type="submit"]');
  wait(
    'document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && !document.querySelector(".space-dialog[open]")',
  );
  assert.equal(
    evaluate(
      'document.querySelector(".direct-select:not(.direct-self) .direct-avatar [data-avatar-id]")?.dataset.avatarId',
    ),
    "799",
    "Accepted DMs show the saved peer avatar, not a generic initial",
  );
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null);
  assert.equal(
    evaluate('document.querySelectorAll("#space-channel-list > li").length'),
    3,
    "DM selection retains the space channels",
  );
  assert.equal(evaluate('document.querySelector(".chat-heading").textContent.includes("#")'), false);
  browser("fill", ".chat-composer textarea", "TEST FIXTURE — a private conversation across spaces.");
  browser("press", "Enter");
  wait('document.querySelector(".chat-messages")?.textContent.includes("a private conversation across spaces.")');
  await control({ incomingMessage: { channelId: "dm0000000001", text: "A live reply from Alex." } });
  wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
  wait('!document.querySelector(".direct-unread")');
  screenshot("dm-desktop");
  browser("click", "#space-channel-list li:first-child .channel-select");
  wait('document.querySelector(".chat-heading")?.textContent.includes("general")');
  assert.equal(
    evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'),
    false,
    "DM history cannot bleed into a channel",
  );
  await control({ incomingMessage: { channelId: "dm0000000001", text: "Unread while browsing a channel." } });
  browser("reload");
  wait('!!document.querySelector(".direct-unread")');
  browser("click", ".direct-select:not(.direct-self)");
  wait(
    'document.querySelector(".chat-messages")?.textContent.includes("Unread while browsing a channel.") && !document.querySelector(".direct-unread")',
  );
  assert.equal((await conversations()).conversations[0].readSeq, "3");
  const dmHistory = await (
    await fetch(`${api}/api/chat/channels/dm0000000001/messages`, {
      headers: { authorization: "Bearer fixture-owner-token", connection: "close" },
    })
  ).json();
  const reactionMessage = dmHistory.messages[0].id;
  await control({ incomingReaction: { channelId: "dm0000000001", messageId: reactionMessage, emoji: "🎉" } });
  wait(
    '!document.querySelector(".chat-initial-messages") && document.querySelector(".chat-reaction")?.getAttribute("aria-label") === "🎉, 1 reaction"',
  );
  wait('fetch("/api/dms").then(r => r.json()).then(v => v.conversations[0].readSeq === "4")');
  assert.equal(
    (await conversations()).conversations[0].readSeq,
    "4",
    "Reactions advance read receipts even though the newest message is still sequence 3",
  );
  browser("scrollintoview", ".chat-reaction");
  evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  browser("click", ".chat-reaction");
  wait(
    'document.querySelector(".chat-reaction")?.getAttribute("aria-pressed") === "true" && document.querySelector(".chat-reaction").getAttribute("aria-disabled") !== "true"',
  );
  assert.equal(evaluate('document.querySelector(".chat-reaction").textContent'), "2");
  screenshot("dm-reactions-desktop");
  browser("click", ".chat-reaction");
  wait(
    'document.querySelector(".chat-reaction")?.getAttribute("aria-pressed") === "false" && document.querySelector(".chat-reaction").getAttribute("aria-disabled") !== "true"',
  );
  assert.equal(evaluate('document.querySelector(".chat-reaction").textContent'), "1");
  browser("reload");
  wait('document.querySelector(".chat-reaction")?.getAttribute("aria-label") === "🎉, 1 reaction"');
  wait('fetch("/api/dms").then(r => r.json()).then(v => v.conversations[0].readSeq === "6")');
  // Reopening the canonical pair must not create a second conversation.
  browser("click", '[aria-label="New direct message"]');
  browser("fill", ".space-dialog input", "@fixture_alex");
  browser("click", '.space-dialog button[type="submit"]');
  wait('!document.querySelector(".space-dialog[open]")');
  assert.equal((await conversations()).conversations.length, 1);
  browser("click", '[aria-label="Create space"]');
  browser("fill", ".space-dialog input", "TEST FIXTURE Second space");
  browser("click", '.space-dialog button[type="submit"]');
  wait(
    'document.querySelector(".space-menu h1")?.textContent === "TEST FIXTURE Second space" && document.querySelector(".chat-heading")?.textContent.includes("general")',
  );
  assert.equal(evaluate('document.querySelectorAll("#space-channel-list > li").length'), 1);
  assert.equal(
    evaluate('document.querySelectorAll(".direct-select").length'),
    2,
    "Self row and the same global DM in a different space",
  );
  browser("click", ".direct-select:not(.direct-self)");
  wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
  assert.equal(
    evaluate('document.querySelector("vite-error-overlay")'),
    null,
    "Switching channel/DM/space must not surface cancelled requests as dev errors",
  );
  screenshot("dm-second-space");
  browser("set", "viewport", "390", "844", "2");
  wait('!!document.querySelector(".navigation-toggle")');
  assert.ok(evaluate("document.documentElement.scrollWidth <= innerWidth"), "Narrow conversation must not overflow");
  assert.equal(
    evaluate('getComputedStyle(document.querySelector(".sidebar-channels")).display'),
    "none",
    "Narrow DM navigation must not crowd the conversation",
  );
  screenshot("dm-narrow");
  browser("click", ".navigation-toggle");
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.equal(
    evaluate(
      'document.querySelector(".direct-select:not(.direct-self) .direct-avatar [data-avatar-id]")?.dataset.avatarId',
    ),
    "799",
    "The narrow sidebar keeps the same peer artwork",
  );
  screenshot("dm-narrow-browse");
  browser("click", ".direct-select:not(.direct-self)");
  wait('!document.querySelector(".spaces-room.navigation-open")');
  browser("fill", ".chat-composer textarea", "A global draft survives losing a space.");
  await control({ noSpaces: true });
  evaluate('window.dispatchEvent(new Event("focus"))');
  wait(
    '!new URL(location.href).searchParams.has("space") && new URL(location.href).searchParams.get("dm") === "dm0000000001" && document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex")',
  );
  assert.equal(
    evaluate('document.querySelector(".chat-composer textarea").value'),
    "A global draft survives losing a space.",
  );
  assert.ok(
    evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'),
    "Space revocation must not clear global DM history",
  );
  browser("open", `${web}/spaces?dm=dm0000000001`);
  wait(
    'document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")',
  );
  screenshot("dm-no-spaces");
  assert.ok(
    evaluate('document.querySelector(".chat-messages").textContent.includes("A live reply from Alex.")'),
    "Global DM survives loss of space membership",
  );
  assert.equal(
    evaluate('document.querySelector(".chat-composer textarea").disabled'),
    false,
    "No-space DMs remain writable",
  );
  browser("fill", ".chat-composer textarea", "TEST FIXTURE — still messaging without a space.");
  browser("press", "Enter");
  wait('document.querySelector(".chat-messages")?.textContent.includes("still messaging without a space.")');
  browser("open", `${web}/spaces`);
  wait('!!document.querySelector(".spaces-empty .direct-select")');
  assert.equal(
    evaluate('document.querySelector(".spaces-empty h1").textContent'),
    "Name your space",
    "Global DMs must preserve first-space onboarding",
  );
  assert.ok(evaluate("document.documentElement.scrollWidth <= innerWidth"));
  screenshot("dm-first-space");
  browser("click", '[aria-label="New direct message"]');
  browser("fill", ".space-dialog input", "fixture_alex");
  browser("click", '.space-dialog button[type="submit"]');
  wait(
    'document.querySelector(".chat-messages")?.textContent.includes("still messaging without a space.") && !document.querySelector(".space-dialog[open]")',
  );
  assert.equal(
    (await conversations()).conversations.length,
    1,
    "No-space onboarding can reopen the existing global DM",
  );
  browser("click", ".navigation-toggle");
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.equal(
    evaluate('document.querySelector(".direct-action").textContent'),
    "New message",
    "No-space accounts cannot invite people to a space",
  );
  await control({
    failure: { path: "/api/dms", method: "POST", status: 503, error: "TEST FIXTURE: notes temporarily unavailable" },
  });
  browser("click", ".direct-self");
  wait('document.querySelector(".direct-section [role=alert]")?.textContent.includes("notes temporarily unavailable")');
  screenshot("dm-self-error");
  assert.equal((await conversations()).conversations.length, 1);
  // Hold the creation request while the user chooses another conversation.
  // A late response may add the notes row, but must not steal that selection.
  evaluate(`(() => {
    const fetch = window.fetch.bind(window);
    window.fetch = async (...args) => {
      if (args[0] === '/api/dms' && args[1]?.method === 'POST') {
        window.fetch = fetch;
        await new Promise(resolve => { window.releaseNotesRequest = resolve; });
      }
      return fetch(...args);
    };
  })()`);
  browser("click", ".direct-self");
  wait('typeof window.releaseNotesRequest === "function" && document.querySelector(".direct-self").disabled');
  browser("click", ".direct-select:not(.direct-self)");
  wait(
    'document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && !document.querySelector(".spaces-room.navigation-open")',
  );
  evaluate("window.releaseNotesRequest()");
  wait('!document.querySelector(".direct-self").disabled');
  assert.equal(
    new URL(evaluate("location.href")).searchParams.get("dm"),
    "dm0000000001",
    "Late notes creation must not override a subsequent selection",
  );
  assert.ok(evaluate('document.querySelector(".chat-heading").textContent.includes("TEST FIXTURE Alex")'));
  browser("click", ".navigation-toggle");
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  browser("click", ".direct-self");
  wait(
    'document.querySelector(".chat-heading")?.textContent.includes("Fixture Owner") && !document.querySelector(".spaces-room.navigation-open")',
  );
  browser("fill", ".chat-composer textarea", "TEST FIXTURE — private notes that survive a reload.");
  browser("press", "Enter");
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  browser("reload");
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  browser("set", "viewport", "1440", "900", "2");
  wait('!!document.querySelector(".direct-self[aria-current=page]")');
  assert.equal(
    evaluate('document.querySelectorAll(".direct-self").length'),
    1,
    "Stored notes must not duplicate the pinned self row",
  );
  screenshot("dm-self-desktop");
  browser("click", ".direct-select:not(.direct-self)");
  wait('document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex")');
  assert.equal(
    evaluate('document.querySelector(".chat-messages").textContent.includes("private notes that survive a reload.")'),
    false,
  );
  browser("click", ".direct-self");
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  const saved = (await conversations()).conversations;
  assert.equal(saved.length, 2, "Reopening self notes must reuse the same conversation");
  assert.equal(saved.find((item) => item.peer.id === "owner0000001").id, "dm0000000002");
  // Each DM keeps its own unsent draft across conversation switches.
  browser("click", ".direct-select:not(.direct-self)");
  wait(
    'document.querySelector(".chat-heading")?.textContent.includes("TEST FIXTURE Alex") && !!document.querySelector(".chat-composer textarea")',
  );
  browser("fill", ".chat-composer textarea", "TEST FIXTURE unsent DM draft");
  browser("click", ".direct-self");
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  assert.equal(evaluate('document.querySelector(".chat-composer textarea").value'), "", "Drafts belong to one DM");
  browser("click", ".direct-select:not(.direct-self)");
  wait('document.querySelector(".chat-composer textarea")?.value === "TEST FIXTURE unsent DM draft"');
  browser("fill", ".chat-composer textarea", "");
  browser("click", ".direct-self");
  wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
  // Instrument only history reads; the real local fixture still supplies data.
  evaluate(`(() => {
    const fetch = window.fetch.bind(window);
    const f = window.historyFixture = { requests: [], hold: false, fail: false };
    window.fetch = async (input, options = {}) => {
      const path = new URL(typeof input === 'string' ? input : input.url, location.href).pathname;
      if (options.method || !/^\\/api\\/chat\\/channels\\/[^/]+\\/messages$/.test(path)) return fetch(input, options);
      const request = { path, aborted: false };
      f.requests.push(request);
      options.signal?.addEventListener('abort', () => { request.aborted = true; request.reason = options.signal.reason.name; }, { once: true });
      if (f.hold) await new Promise((resolve, reject) => {
        request.release = resolve;
        options.signal?.addEventListener('abort', () => reject(options.signal.reason), { once: true });
      });
      if (f.fail) return Response.json({ error: 'TEST FIXTURE: history unavailable' }, { status: 503 });
      return fetch(input, options);
    };
  })()`);
  for (const width of [1440, 390]) {
    browser("set", "viewport", String(width), "844", "2");
    const select = (selector) => {
      if (width === 390) {
        browser("click", ".navigation-toggle");
        wait('!!document.querySelector(".spaces-room.navigation-open")');
      }
      browser("click", selector);
    };
    evaluate("historyFixture.requests = []");
    select(".direct-select:not(.direct-self)");
    wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
    // Development StrictMode replays mount effects; its abandoned read must be
    // cancelled. Production has exactly one read, with no replay to ignore.
    assert.equal(
      evaluate("historyFixture.requests.filter(r => !r.aborted).length"),
      1,
      "Opening a DM leaves only one history read",
    );
    evaluate("historyFixture.requests = []; historyFixture.hold = true");
    select(".direct-self");
    wait(
      'historyFixture.requests.length > 0 && document.querySelector(".chat-messages [role=status]")?.textContent === "Loading messages…"',
    );
    evaluate("historyFixture.hold = false");
    select(".direct-select:not(.direct-self)");
    wait('document.querySelector(".chat-messages")?.textContent.includes("A live reply from Alex.")');
    assert.equal(
      evaluate(
        'historyFixture.requests.filter(r => r.path.includes("dm0000000002")).every(r => r.aborted && r.reason === "AbortError")',
      ),
      true,
      "All abandoned history reads are cancelled by navigation, not timeout",
    );
    assert.deepEqual(
      evaluate("historyFixture.requests.filter(r => !r.aborted).map(r => r.path)"),
      ["/api/chat/channels/dm0000000001/messages"],
      "The current history read stays alive",
    );
    evaluate("historyFixture.requests.forEach(r => r.release?.())");
    assert.equal(
      evaluate('document.querySelector(".chat-messages").textContent.includes("private notes that survive a reload.")'),
      false,
    );
    evaluate("historyFixture.requests = []; historyFixture.fail = true");
    select(".direct-self");
    wait('document.querySelector(".chat-state[role=alert]")?.textContent.includes("history unavailable")');
    assert.equal(
      evaluate("historyFixture.requests.filter(r => !r.aborted).length"),
      1,
      "A failed load does not trigger a second implicit history read",
    );
    evaluate("historyFixture.fail = false");
    browser("find", "role", "button", "click", "--name", "Try again", "--exact");
    wait('document.querySelector(".chat-messages")?.textContent.includes("private notes that survive a reload.")');
    assert.equal(
      evaluate("historyFixture.requests.filter(r => !r.aborted).length"),
      2,
      "Retry issues exactly one fresh history read",
    );
    assert.equal(evaluate('document.querySelector("vite-error-overlay")'), null);
    console.log(
      `PASS: one uncancelled DM history read per open, cancellation on rapid switch, isolated current history, and explicit failure/retry at ${width}px.`,
    );
  }
  browser("set", "viewport", "390", "844", "2");
  browser("click", ".navigation-toggle");
  wait('!!document.querySelector(".spaces-room.navigation-open")');
  assert.ok(evaluate("document.documentElement.scrollWidth <= innerWidth"));
  screenshot("dm-self-narrow-browse");
  console.log(
    "PASS: self notes lazy creation/error/reuse/send/persistence/isolation/navigation race, full-width dividers, real space invitation action, DM start/error/reopen/unsent drafts/live replies/reactions/read receipts, space independence, and desktop/narrow navigation (disposable fixture).",
  );
} finally {
  browser("close");
}
