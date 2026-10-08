// Disposable browser mocks: hold chat-session responses across cold/cached navigation.
// Start Vite, then run with MESSAGE_TEST_CHROME pointing to a fine-pointer Chromium wrapper.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const web = process.env.MESSAGE_TEST_WEB_URL ?? "http://localhost:5174";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(web).hostname));
const directory = mkdtempSync(join(tmpdir(), "caper-space-reactions-"));
const init = join(directory, "fixture.js");
const artifacts = process.env.MESSAGE_TEST_ARTIFACTS && resolve(process.env.MESSAGE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });

function fixture() {
  if (location.protocol === "about:") return;
  const account = { id: "member123456", username: "member", displayName: "TEST FIXTURE member" };
  const spaces = [
    { id: "space1234567", name: "TEST FIXTURE Studio", ownerId: "owner1234567" },
    { id: "space2345678", name: "TEST FIXTURE Lounge", ownerId: "owner1234567" },
  ];
  const channels = spaces.map((space, index) => ({
    id: `channel1234${index}`,
    spaceId: space.id,
    name: "general",
    private: false,
    joined: true,
  }));
  channels.push({ ...channels[1], id: "channel12342", name: "design" });
  const control = (window.reactionFixture = { releases: [], writes: 0, badFrames: [], frames: 0 });
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    constructor(url, protocols) {
      super();
      if (!String(url).includes("/api/chat/events")) return new NativeSocket(url, protocols);
      queueMicrotask(() => this.frame({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === "heartbeat") this.frame({ type: "heartbeat" });
      if (request.type !== "subscribe") return;
      const event =
        request.kind === "chat"
          ? { type: "ready", cursor: request.after ?? "2" }
          : request.kind === "presence"
            ? { type: "snapshot", members: request.userIds.map((userId) => ({ userId, status: "online" })) }
            : { type: "snapshot", participants: [], revision: 1 };
      this.frame({ type: "event", id: request.id, event });
      this.frame({ type: "subscribed", id: request.id });
    }
    frame(value) {
      this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) }));
    }
    close() {}
  };
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (!path.startsWith("/api/")) return originalFetch(input, options);
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/spaces")
      return Response.json({
        spaces,
        invitations: [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    const space = spaces.find((item) => path === `/api/spaces/${item.id}`);
    if (space)
      return Response.json({
        space,
        channels: channels.filter((item) => item.spaceId === space.id),
        members: [{ ...account, owner: false }],
      });
    if (path === "/api/dms") return Response.json({ conversations: [] });
    if (path === "/api/blocks") return Response.json({ accounts: [] });
    if (path === "/api/chat/session") {
      const status = await new Promise((resolve) => control.releases.push(resolve));
      return status === 503
        ? Response.json({ error: "TEST FIXTURE session unavailable" }, { status })
        : Response.json({ token: "fixture", author: { id: account.id, name: account.displayName, isGuest: false } });
    }
    if (path.endsWith("/media/status")) return Response.json({ enabled: false });
    if (path.endsWith("/reactions") && options.method === "PUT") control.writes++;
    const channel = channels.find((item) => path === `/api/chat/channels/${item.id}/messages`);
    if (channel)
      return Response.json({
        space: spaces.find((space) => space.id === channel.spaceId),
        channel,
        cursor: "2",
        hasMore: false,
        messages: [1, 2].map((index) => ({
          id: `${channel.id}-${index}`,
          channelId: channel.id,
          seq: String(index),
          clientMessageId: `${channel.id}-${index}`,
          createdAt: "2026-10-08T00:00:00Z",
          author: { id: "other", name: "TEST FIXTURE teammate", isGuest: false },
          content: {
            version: 1,
            type: "text",
            text: `TEST FIXTURE ${channel.name}: ${index === 1 ? "You and a teammate reacted below." : "Only your teammate reacted below."}`,
          },
          reactions: [{ emoji: index === 1 ? "🇩🇴" : "🚀", authorIds: index === 1 ? ["other", account.id] : ["other"] }],
        })),
      });
    return Response.json({ error: "TEST FIXTURE: disabled endpoint" }, { status: 503 });
  };
  // Sample every rendered frame, including the initial preview/virtual-list handoff.
  const inspect = () => {
    for (const chip of document.querySelectorAll(".chat-reaction")) {
      control.frames++;
      const expected = chip.querySelector("img").alt === "🇩🇴";
      if (chip.getAttribute("aria-pressed") !== String(expected)) control.badFrames.push("wrong ownership");
      if (getComputedStyle(chip).opacity !== "1") control.badFrames.push("dimmed reaction");
    }
    requestAnimationFrame(inspect);
  };
  requestAnimationFrame(inspect);
}

writeFileSync(init, `(${fixture.toString()})()`);
const args = ["--session", "space-reactions", "--init-script", init];
if (process.env.MESSAGE_TEST_CHROME) args.push("--executable-path", process.env.MESSAGE_TEST_CHROME);
const browser = (...command) => {
  const result = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60_000 }),
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
const check = (disabled) => {
  assert.deepEqual(
    evaluate(`(() => {
    const chips = [...document.querySelectorAll('.chat-reaction')];
    return chips.map(chip => [chip.getAttribute('aria-pressed'), chip.getAttribute('aria-disabled')]);
  })()`),
    [
      ["true", String(disabled)],
      ["false", String(disabled)],
    ],
  );
  assert.deepEqual(evaluate("reactionFixture.badFrames"), []);
  assert.ok(evaluate("reactionFixture.frames") > 0);
  // Settle mouse/focus styles before asserting idle visibility, separately from
  // the frame-by-frame ownership check. Hover/focus is exercised below.
  evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  assert.equal(
    evaluate(
      '[...document.querySelectorAll(".chat-add-reaction")].every(button => getComputedStyle(button).opacity === "0" || getComputedStyle(button).display === "none")',
    ),
    true,
  );
};
const loaded = (channel) =>
  wait(
    `document.querySelector('[data-message-key="${channel}-1"] .chat-reaction') && !document.querySelector('.chat-initial-messages') && reactionFixture.releases.length > 0`,
  );

try {
  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  browser("open", `${web}/spaces?space=space1234567&channel=channel12340`);
  assert.equal(
    evaluate('matchMedia("(hover: hover) and (pointer: fine)").matches'),
    true,
    "Use a fine-pointer Chromium wrapper",
  );
  loaded("channel12340");
  check(true);
  evaluate("reactionFixture.releases.splice(0).forEach(release => release(200))");
  wait('document.querySelector(".chat-reaction").getAttribute("aria-disabled") === "false"');
  check(false);

  for (const [space, channel] of [
    ["space2345678", "channel12341"],
    ["space1234567", "channel12340"],
    ["space2345678", "channel12341"],
  ]) {
    browser("click", `.space-rail button[title="TEST FIXTURE ${space === "space1234567" ? "Studio" : "Lounge"}"]`);
    loaded(channel);
    check(true);
    browser("click", ".chat-reaction");
    assert.equal(evaluate("reactionFixture.writes"), 0, "Knowing ownership must not allow writes without a session");
    browser("click", "#chat-heading");
    screenshot("reactions-desktop-switch");
    evaluate("reactionFixture.releases.splice(0).forEach(release => release(200))");
    wait('document.querySelector(".chat-reaction").getAttribute("aria-disabled") === "false"');
    check(false);
  }
  browser("hover", '[data-message-key="channel12341-1"] p');
  wait('getComputedStyle(document.querySelector(".chat-add-reaction")).opacity === "1"');
  assert.equal(evaluate('getComputedStyle(document.querySelector(".chat-add-reaction")).opacity'), "1");
  screenshot("reactions-desktop-hover");
  browser("mouse", "move", "5", "5");
  browser("focus", ".chat-add-reaction");
  assert.equal(evaluate('getComputedStyle(document.querySelector(".chat-add-reaction")).opacity'), "1");
  browser("press", "Enter");
  wait('!!document.querySelector(".chat-reaction-picker")');
  browser("press", "Escape");
  wait('!document.querySelector(".chat-reaction-picker")');

  // Same-space switch with a failed session still has correct ownership/visibility.
  browser("click", '.channel-select:not([aria-current="page"])');
  loaded("channel12342");
  evaluate("reactionFixture.releases.splice(0).forEach(release => release(503))");
  wait('document.body.textContent.includes("TEST FIXTURE session unavailable")');
  check(true);
  browser("find", "role", "button", "click", "--name", "Retry session", "--exact");
  wait("reactionFixture.releases.length > 0");
  evaluate("reactionFixture.releases.splice(0).forEach(release => release(200))");
  wait('document.querySelector(".chat-reaction").getAttribute("aria-disabled") === "false"');
  check(false);

  browser("set", "viewport", "390", "844", "2");
  browser("reload");
  loaded("channel12342");
  check(true);
  evaluate("reactionFixture.releases.splice(0).forEach(release => release(200))");
  browser("click", ".navigation-toggle");
  browser("click", '.space-rail button[title="TEST FIXTURE Studio"]');
  loaded("channel12340");
  check(true);
  assert.equal(
    evaluate(
      '[...document.querySelectorAll(".chat-add-reaction")].every(button => getComputedStyle(button).display === "none")',
    ),
    true,
  );
  assert.equal(evaluate("document.documentElement.scrollWidth <= innerWidth"), true);
  screenshot("reactions-narrow-switch");
  console.log(
    "PASS: first-frame own/other reactions, cold/cached space switches, channel switch, held/failed/retried sessions, no premature writes, desktop hover/focus/picker, narrow layout; no incorrect rendered frames.",
  );
} finally {
  browser("close");
  rmSync(directory, { recursive: true, force: true });
}
