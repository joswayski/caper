// Focused browser regression with disposable API/gateway mocks; never contacts production.
// NAVIGATION_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-desktop-navigation.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const base = process.env.NAVIGATION_TEST_WEB_URL ?? "http://localhost:5174/spaces";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(base).hostname), "Use a loopback dev server");
const url = new URL(base);
url.searchParams.set("space", "space1234567");
url.searchParams.set("channel", "general12345");
const directory = mkdtempSync(join(tmpdir(), "caper-navigation-"));
const init = join(directory, "fixture.js");

function fixture() {
  if (location.protocol === "about:") return;
  const account = { id: "member123456", username: "member", displayName: "TEST FIXTURE member" };
  const space = { id: "space1234567", name: "TEST FIXTURE navigation", ownerId: "owner1234567" };
  const channels = [
    { id: "general12345", spaceId: space.id, name: "general", private: false, joined: true },
    { id: "design123456", spaceId: space.id, name: "design", private: false, joined: true },
  ];
  const members = [{ ...account, owner: false }];
  const control = (window.navigationFixture = {
    historyReads: {},
    presenceSubscribes: 0,
    releases: [],
    holdDesign: true,
    directReleases: [],
    directError: "dm_not_accepted",
  });
  const NativeSocket = window.WebSocket;
  window.WebSocket = class extends EventTarget {
    constructor(socketUrl, protocols) {
      super();
      if (!String(socketUrl).includes("/api/chat/events")) return new NativeSocket(socketUrl, protocols);
      queueMicrotask(() => this.frame({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data) {
      const request = JSON.parse(data);
      if (request.type === "heartbeat") this.frame({ type: "heartbeat" });
      if (request.type !== "subscribe") return;
      if (request.kind === "presence") control.presenceSubscribes++;
      const event =
        request.kind === "chat" || request.kind === "notifications"
          ? { type: "ready", cursor: request.after ?? "1" }
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
        spaces: [space],
        invitations: [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path === `/api/spaces/${space.id}`) return Response.json({ space, channels, members });
    if (path === "/api/dms") {
      if (options.method !== "POST") return Response.json({ conversations: [] });
      await new Promise((resolve) => control.directReleases.push(resolve));
      return Response.json({ error: "TEST FIXTURE refusal", code: control.directError }, { status: 403 });
    }
    if (path === "/api/chat/session")
      return Response.json({ token: "fixture", author: { id: account.id, name: account.displayName, isGuest: false } });
    if (path.endsWith("/media/status")) return Response.json({ enabled: false });
    const match = path.match(/\/api\/chat\/channels\/([^/]+)\/messages$/);
    if (match) {
      const channel = channels.find((item) => item.id === match[1]);
      control.historyReads[channel.id] = (control.historyReads[channel.id] ?? 0) + 1;
      if (channel.id === "design123456" && control.holdDesign)
        await new Promise((resolve) => control.releases.push(resolve));
      return Response.json({
        space,
        channel,
        cursor: "1",
        hasMore: false,
        messages: [
          {
            id: `message-${channel.id}`,
            channelId: channel.id,
            seq: "1",
            clientMessageId: `00000000-0000-4000-8000-${channel.id === "general12345" ? "000000000001" : "000000000002"}`,
            createdAt: "2026-10-04T12:00:00Z",
            author: { id: account.id, name: account.displayName, isGuest: false },
            content: { version: 1, type: "text", text: `TEST FIXTURE ${channel.name} history` },
          },
        ],
      });
    }
    return Response.json({ error: "TEST FIXTURE: disabled endpoint" }, { status: 503 });
  };
}

writeFileSync(init, `(${fixture.toString()})()`);
const session = `nav-${process.pid}`;
const args = ["--session", session, "--init-script", init];
const browser = (...command) => {
  const output = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60_000 }),
  );
  assert.ok(output.success, output.error);
  return output.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (expression) => browser("wait", "--fn", expression);
const artifacts = process.env.NAVIGATION_TEST_MOBILE_ARTIFACTS;
if (artifacts) mkdirSync(artifacts, { recursive: true });
const screenshot = (name) => {
  if (artifacts) browser("screenshot", `${artifacts}/${name}.png`);
};
let socket;

try {
  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  browser("open", url.toString());
  wait('document.querySelector("#chat-message") && !document.querySelector("#chat-message").disabled');
  evaluate("document.fonts.ready.then(() => true)");
  for (const width of [900, 1920, 1280]) {
    browser("set", "viewport", String(width), "900", "2");
    for (const draft of ["", "One line", "First line\nSecond line\nThird line"]) {
      if (draft) browser("fill", "#chat-message", draft);
      else {
        browser("click", "#chat-message");
        browser("press", "Control+a");
        browser("press", "Backspace");
      }
      evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
      assert.equal(evaluate('document.querySelector("#chat-message").value'), draft);
      const bounds = evaluate(`(() => {
        const panel = document.querySelector(".voice-panel");
        const account = panel.getBoundingClientRect();
        const field = document.querySelector("#chat-message").getBoundingClientRect();
        const composer = document.querySelector(".chat-composer").getBoundingClientRect();
        const dividerTop = account.top + parseFloat(getComputedStyle(panel).borderTopWidth)
          + parseFloat(getComputedStyle(panel, "::before").top);
        return [account.top - field.top, account.bottom - field.bottom, dividerTop - composer.top];
      })()`);
      if (draft.includes("\n")) {
        assert.equal(bounds[1], 0, `Multiline input keeps the account bar's bottom edge at ${width}px`);
        assert.ok(bounds[0] > 0, "Multiline input must still grow upward");
        if (width === 1280) screenshot("desktop-composer-multiline");
      } else {
        assert.deepEqual(bounds, [0, 0, 0], `Account/input outlines and upper dividers align at ${width}px`);
        if (draft === "") screenshot(`desktop-composer-aligned-${width}`);
      }
    }
  }
  browser("fill", "#chat-message", "");
  console.log(
    "PASS: account/input top, bottom and divider offsets are 0px at 900px, 1280px and 1920px; multiline grows upward without moving their bottom edge.",
  );
  // Wide layouts show the member list by default.
  assert.equal(evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded")'), "true");
  wait('document.querySelector(".space-member-presence .presence-dot")?.dataset.status === "online"');
  screenshot("desktop-members-default");
  browser("fill", "#chat-message", "draft survives repeat click");
  browser("click", ".member-list-toggle");
  wait('!document.querySelector(".space-member-presence")');
  assert.equal(evaluate('document.querySelector("#chat-message").value'), "draft survives repeat click");
  screenshot("desktop-members-hidden");
  browser("click", ".member-list-toggle");
  wait('document.querySelector(".space-member-presence .presence-dot")?.dataset.status === "online"');
  evaluate('(window.navigationFixture.messageNode = document.querySelector(".chat-message"), true)');
  const initialPresence = evaluate("navigationFixture.presenceSubscribes");
  const initialGeneralReads = evaluate("navigationFixture.historyReads.general12345");
  browser("click", '[aria-current="page"].channel-select');
  assert.equal(evaluate('document.querySelector("#chat-message").value'), "draft survives repeat click");
  assert.equal(
    evaluate('document.querySelector(".chat-message") === navigationFixture.messageNode'),
    true,
    "repeat click must retain rendered history nodes",
  );
  assert.equal(
    evaluate("navigationFixture.historyReads.general12345"),
    initialGeneralReads,
    "repeat click must not refetch history",
  );

  browser("hover", '.channel-select:not([aria-current="page"])');
  wait("navigationFixture.historyReads.design123456 === 1");
  browser("click", '.channel-select:not([aria-current="page"])');
  assert.equal(
    evaluate("navigationFixture.historyReads.design123456"),
    1,
    "hover and click must share the in-flight history request",
  );
  browser("click", '[aria-current="page"].channel-select');
  wait('!document.querySelector("[aria-busy=true].channel-select")');
  evaluate("navigationFixture.releases.splice(0).forEach(release => release())");
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(
    evaluate('document.querySelector("#space-channel-list [aria-current=page] span").textContent'),
    "general",
    "stale completion must not replace the restored channel",
  );
  assert.equal(
    evaluate('document.querySelector("#chat-message").value'),
    "draft survives repeat click",
    "canceling pending navigation must retain the visible draft",
  );

  evaluate("navigationFixture.holdDesign = false");
  browser("click", '.channel-select:not([aria-current="page"])');
  wait(
    'document.querySelector("#space-channel-list [aria-current=page] span").textContent === "design" && document.body.textContent.includes("TEST FIXTURE design history")',
  );
  assert.equal(
    evaluate("navigationFixture.presenceSubscribes"),
    initialPresence,
    "same-space channel changes must not restart presence subscriptions",
  );
  assert.equal(
    evaluate("navigationFixture.historyReads.design123456"),
    2,
    "a canceled request is not treated as a committed visited history",
  );
  // Each conversation keeps its own unsent text across switches.
  assert.equal(evaluate('document.querySelector("#chat-message").value'), "", "drafts belong to one conversation");
  browser("fill", "#chat-message", "design draft");
  browser("click", '.channel-select:not([aria-current="page"])');
  wait(
    'document.querySelector("#space-channel-list [aria-current=page] span").textContent === "general" && document.querySelector("#chat-message")?.value === "draft survives repeat click"',
  );
  browser("click", '.channel-select:not([aria-current="page"])');
  wait(
    'document.querySelector("#space-channel-list [aria-current=page] span").textContent === "design" && document.querySelector("#chat-message")?.value === "design draft"',
  );
  if (process.env.NAVIGATION_TEST_SCREENSHOT) browser("screenshot", process.env.NAVIGATION_TEST_SCREENSHOT);

  browser("set", "viewport", "390", "844", "2");
  evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
  assert.equal(evaluate("document.documentElement.scrollWidth <= innerWidth"), true);
  // Narrowing closes the desktop member column instead of opening a phone drawer.
  wait('!document.querySelector(".space-member-presence")');
  assert.equal(evaluate('document.body.textContent.includes("TEST FIXTURE design history")'), true);
  assert.equal(
    evaluate('getComputedStyle(document.querySelector(".chat-title-menu")).display'),
    "block",
    "The mobile channel title opens the channel menu",
  );
  assert.deepEqual(
    evaluate(
      '["chat-pins-toggle", "member-list-toggle"].map(name => getComputedStyle(document.querySelector(`.chat-heading .${name}`)).display)',
    ),
    ["none", "none"],
    "Mobile Pins and Members move into the channel menu",
  );
  // The desktop Pins control stays mounted but is hidden on phones.
  assert.equal(
    evaluate('document.querySelectorAll(".chat-heading .chat-pins-toggle").length'),
    1,
    "There is only one desktop Pins control",
  );
  assert.equal(evaluate('document.querySelector(".navigation-toggle").getAttribute("aria-label")'), "Back to Browse");
  assert.equal(evaluate('document.querySelector(".navigation-toggle").getBoundingClientRect().width'), 44);

  // Send real Chromium touch input through the agent-browser session's CDP.
  socket = new WebSocket(browser("get", "cdp-url").cdpUrl);
  await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const response = JSON.parse(event.data),
      request = pending.get(response.id);
    if (!request) return;
    pending.delete(response.id);
    if (response.error) request.reject(new Error(response.error.message));
    else request.resolve(response.result);
  });
  const cdp = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++nextId;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
  const { targetInfos } = await cdp("Target.getTargets");
  const target = targetInfos.find((target) => target.type === "page" && target.url.startsWith(base));
  assert.ok(target);
  const { sessionId } = await cdp("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 2 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  // Touches carry their own timestamps (seconds), `after` the previous one, so
  // swipe speeds do not depend on how quickly headless Chromium renders.
  let clock = 0;
  const touch = (type, x, y, after = 0.016) => {
    clock = type === "touchStart" ? Math.max(clock, Date.now() / 1000) : clock + after;
    return cdp(
      "Input.dispatchTouchEvent",
      { type, timestamp: clock, touchPoints: type === "touchEnd" || type === "touchCancel" ? [] : [{ x, y }] },
      sessionId,
    );
  };
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  // Moves 16ms apart over `duration`, like a finger; then the slide settles.
  const swipe = async (x, y, dx, dy = 0, end = "touchEnd", duration = 96) => {
    const steps = Math.max(1, Math.round(duration / 16));
    await touch("touchStart", x, y);
    for (let step = 1; step <= steps; step++)
      await touch("touchMove", x + (dx * step) / steps, y + (dy * step) / steps);
    await touch(end);
    await pause(500);
  };
  const browsing = () => evaluate('document.querySelector(".spaces-room").classList.contains("navigation-open")');
  const settled = () =>
    wait(
      '!document.querySelector(".spaces-room").hasAttribute("data-browse-slide") && !document.querySelector(".spaces-room > .stage").getAnimations().length',
    );
  const point = (selector) =>
    evaluate(
      `(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`,
    );
  browser("fill", "#chat-message", "mobile draft survives both swipe directions");
  browser("click", ".chat-heading"); // Dismiss the keyboard/focus without clearing the draft.
  const currentUrl = evaluate("location.href");
  const reads = evaluate("navigationFixture.historyReads.design123456");
  const p = point(".chat-messages");
  for (const [dx, dy, end, duration] of [
    [60, 0, "touchEnd", 800],
    [120, 0, "touchEnd", 1200],
    [30, 0],
    [90, 140],
    [100, 0, "touchCancel"],
  ]) {
    await swipe(p.x - 60, p.y, dx, dy, end, duration);
    settled();
    assert.equal(
      browsing(),
      false,
      "Slow drags short of halfway, tiny flicks, wrong-direction, vertical and canceled gestures snap back",
    );
  }
  assert.equal(
    evaluate('document.querySelector("#chat-message").value'),
    "mobile draft survives both swipe directions",
  );

  // The conversation follows the finger over a still, live Browse. It travels
  // past the screen's edge, so its shadow leaves too.
  const travel = evaluate(
    'document.documentElement.clientWidth - document.querySelector(".spaces-room").getBoundingClientRect().left + 48',
  );
  await touch("touchStart", p.x - 100, p.y);
  for (let step = 1; step <= 10; step++) await touch("touchMove", p.x - 100 + step * 10, p.y);
  wait('document.querySelector(".spaces-room > .stage").getAnimations().length === 1');
  assert.equal(evaluate('document.querySelector(".spaces-room").hasAttribute("data-browse-slide")'), true);
  assert.equal(browsing(), true, "Browse renders live beneath the sliding conversation");
  const scrubbed = evaluate(
    'document.querySelector(".spaces-room > .stage").getAnimations().map(a => [a.playState, Math.round(a.currentTime)])',
  );
  for (const [state, time] of scrubbed) {
    assert.equal(state, "paused");
    assert.ok(Math.abs(time - (100 / travel) * 1000) < 15, `slide follows the finger (${time})`);
  }
  screenshot("mobile-swipe-midway");
  await touch("touchEnd", 0, 0, 0.2); // Held still before lifting: not a fling.
  settled();
  assert.equal(browsing(), false, "Releasing before halfway snaps back to the conversation");

  const fingers = [
    { id: 0, x: p.x - 60, y: p.y },
    { id: 1, x: p.x - 60, y: p.y + 40 },
  ];
  await cdp("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: fingers }, sessionId);
  await cdp(
    "Input.dispatchTouchEvent",
    { type: "touchMove", touchPoints: fingers.map((finger) => ({ ...finger, x: finger.x + 100 })) },
    sessionId,
  );
  await touch("touchEnd");
  assert.equal(browsing(), false, "Multi-touch must not trigger one-finger navigation");
  const composer = point("#chat-message");
  await swipe(composer.x - 50, composer.y, 100);
  assert.equal(browsing(), false, "Editing/text selection in the composer must not navigate");
  for (const selector of [".voice-panel .voice-icon-button", ".chat-composer .chat-attach"]) {
    const control = point(selector);
    await swipe(control.x, control.y, 100);
    settled();
    assert.equal(browsing(), false, "Audio and composer buttons keep their own gestures");
  }
  evaluate("window.getSelection()?.removeAllRanges(); document.activeElement.blur()");
  screenshot("mobile-chat");
  await swipe(p.x, p.y, 100);
  settled();
  assert.equal(browsing(), true, "A right flick starting at the screen center reveals Browse");
  // Browse takes the room; the conversation is hidden entirely, not kept at the edge.
  const hidden = evaluate(`(() => {
    const room = document.querySelector(".spaces-room").getBoundingClientRect();
    const stage = document.querySelector(".spaces-room > .stage");
    return {
      visibility: getComputedStyle(stage).visibility,
      browse: room.right - document.querySelector(".sidebar-channels").getBoundingClientRect().right,
      inert: stage.inert,
    };
  })()`);
  assert.equal(hidden.visibility, "hidden", "No part of the conversation shows beside Browse");
  assert.ok(hidden.browse <= 16, `Browse fills the width (${hidden.browse})`);
  assert.equal(hidden.inert, true, "The hidden conversation is not interactive");
  screenshot("mobile-browse");
  const row = point('.channel-select:not([aria-current="page"])');
  await swipe(row.x, row.y, -110);
  settled();
  assert.equal(browsing(), false, "A center-start left swipe on a channel row returns to chat without selecting it");
  const pinsButton = point(".chat-pins-toggle");
  await swipe(pinsButton.x, pinsButton.y, 64);
  settled();
  assert.equal(browsing(), true, "Header buttons can start navigation swipes");
  assert.equal(evaluate('!!document.querySelector(".chat-pins")'), false, "A swipe must not click its start button");
  await swipe(row.x, row.y, -110);
  settled();
  assert.equal(browsing(), false);
  await swipe(p.x - 100, p.y, 220, 0, "touchEnd", 1200);
  settled();
  assert.equal(browsing(), true, "A slow drag past halfway reveals Browse");
  await swipe(row.x + 40, row.y, -200, 0, "touchEnd", 1200);
  settled();
  assert.equal(browsing(), false, "A slow drag back past halfway returns to the conversation");
  assert.equal(evaluate("location.href"), currentUrl);
  assert.equal(
    evaluate('document.querySelector("#chat-message").value'),
    "mobile draft survives both swipe directions",
  );
  assert.equal(
    evaluate("navigationFixture.historyReads.design123456"),
    reads,
    "Showing and hiding Browse must not refetch the current history",
  );

  assert.equal(
    evaluate('document.querySelector(".chat-heading .chat-pins-toggle").getAttribute("aria-label")'),
    "Pins",
  );
  browser("click", ".chat-title-menu > summary");
  browser("click", ".chat-header-menu button:first-child");
  wait('document.querySelector(".chat-pins")?.textContent.includes("No pinned messages.")');
  screenshot("mobile-empty-pins");
  await swipe(p.x - 60, p.y, 100);
  assert.equal(browsing(), false, "Open Pins must not pass swipes through");
  browser("press", "Escape");
  wait('!document.querySelector(".chat-pins") && !!document.querySelector(".chat-message")');
  assert.equal(
    evaluate('document.querySelector("#chat-message").value'),
    "mobile draft survives both swipe directions",
    "Pins must preserve the conversation draft",
  );
  // Members slides over the conversation like Browse and follows a finger both
  // ways: left from the conversation opens it, right on the panel or backdrop closes it.
  const drawer = ".space-member-presence";
  const drawerOpen = () => evaluate(`!!document.querySelector("${drawer}:not([inert])")`);
  const drawerSettled = () => wait(`!document.querySelector("${drawer}")?.getAnimations().length`);
  const drawerGone = () => wait(`!document.querySelector("${drawer}")`);
  browser("click", ".chat-title-menu > summary");
  // Safari may leave focus nowhere when a menu item is tapped. That blur must
  // not hide the item before the click has a chance to open Members.
  evaluate('document.querySelector(".chat-title-menu > summary").focus(); document.activeElement.blur()');
  assert.equal(
    evaluate('document.querySelector(".chat-title-menu").open'),
    true,
    "A null-focus blur keeps the menu open",
  );
  assert.equal(
    evaluate(`(async () => {
      document.querySelector(".chat-header-menu button[aria-expanded]").click();
      await new Promise((resolve) => setTimeout(resolve));
      return document.querySelector("${drawer}").getAnimations().length;
    })()`),
    1,
    "Members slides in",
  );
  drawerSettled();
  assert.equal(evaluate('document.querySelector(".chat-title-menu").open'), false, "Choosing Members closes the menu");
  assert.equal(
    evaluate('document.querySelector(".chat-header-menu button[aria-expanded]").getAttribute("aria-expanded")'),
    "true",
  );
  screenshot("mobile-members");
  await swipe(20, p.y, 0, -100);
  assert.equal(drawerOpen(), true, "A vertical gesture on the backdrop must not close Members");
  await swipe(20, p.y, 100);
  drawerGone();
  assert.equal(browsing(), false, "The member backdrop must not pass swipes through");
  assert.equal(drawerOpen(), false, "A right swipe on the backdrop closes Members");
  await swipe(p.x, p.y, -100);
  drawerSettled();
  assert.equal(drawerOpen(), true, "A center-start left swipe opens Members");
  const memberRow = point(`${drawer} li`);
  await swipe(memberRow.x, memberRow.y, 60, 0, "touchEnd", 800);
  drawerSettled();
  assert.equal(drawerOpen(), true, "A slow drag short of halfway leaves Members open");
  await swipe(memberRow.x, memberRow.y, 100);
  drawerGone();
  assert.equal(browsing(), false, "A right flick on the panel closes Members without opening Browse");
  await touch("touchStart", p.x + 100, p.y);
  for (let step = 1; step <= 10; step++) await touch("touchMove", p.x + 100 - step * 10, p.y);
  // CDP acknowledges a move before Chromium delivers its final pointer frame.
  evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
  const [offset, drawerTravel] = evaluate(`(() => {
    const room = document.querySelector(".spaces-room"), panel = document.querySelector("${drawer}");
    return [new DOMMatrixReadOnly(getComputedStyle(panel).transform).m41, room.clientWidth - panel.offsetLeft + 48];
  })()`);
  assert.ok(Math.abs(offset - (drawerTravel - 100)) < 2, `Members follows the finger (${offset})`);
  screenshot("mobile-members-swipe-midway");
  await touch("touchEnd", 0, 0, 0.2); // Held still before lifting: not a fling.
  drawerGone();
  await swipe(p.x + 100, p.y, -100);
  drawerSettled();
  assert.equal(drawerOpen(), true, "A left flick on the conversation opens Members");
  assert.equal(browsing(), false);
  browser("click", ".member-list-close");
  drawerGone();
  // Check the moving frames, not just the destination: a snap also opens Browse.
  const buttonSlide = (selector, opening) => {
    const result = evaluate(`(async () => {
      const room = document.querySelector(".spaces-room"), stage = room.querySelector(":scope > .stage");
      const travel = document.documentElement.clientWidth - room.getBoundingClientRect().left + 48;
      const positions = [];
      let duration;
      document.querySelector(${JSON.stringify(selector)}).click();
      for (let frame = 0; frame < 120; frame++) {
        await new Promise(requestAnimationFrame);
        const animation = stage.getAnimations()[0];
        if (animation) {
          duration = animation.effect.getTiming().duration;
          positions.push(new DOMMatrixReadOnly(getComputedStyle(stage).transform).m41);
        }
        if (!room.hasAttribute("data-browse-slide")) break;
      }
      return { duration, positions, travel };
    })()`);
    assert.equal(result.duration, 300, "Button slides take the steady 300ms ease-out");
    assert.ok(
      result.positions.some((x) => x > 1 && x < result.travel - 1),
      "Button navigation must show intermediate positions",
    );
    const movement = result.positions.at(-1) - result.positions[0];
    assert.ok(opening ? movement > 1 : movement < -1, "The conversation must slide in the requested direction");
    settled();
  };
  buttonSlide(".navigation-toggle", true);
  assert.equal(browsing(), true, "Back button must open Browse");
  assert.equal(evaluate('!!document.querySelector(".browse-peek")'), false, "No conversation edge stays in view");
  buttonSlide('.channel-select[aria-current="page"]', false);
  assert.equal(browsing(), false);

  // Reduced motion keeps the instant switch and its quick-swipe threshold.
  await cdp(
    "Emulation.setEmulatedMedia",
    { features: [{ name: "prefers-reduced-motion", value: "reduce" }] },
    sessionId,
  );
  await swipe(p.x - 60, p.y, 68);
  assert.equal(
    evaluate('document.querySelector(".spaces-room").hasAttribute("data-browse-slide")'),
    false,
    "No slide with reduced motion",
  );
  assert.equal(browsing(), true, "Reduced motion: a quick swipe still reveals Browse");
  browser("click", '.channel-select[aria-current="page"]');
  assert.equal(browsing(), false, "Reduced motion: choosing a channel returns at once");
  await cdp("Emulation.setEmulatedMedia", { features: [] }, sessionId);
  assert.equal(
    evaluate('document.querySelector("#chat-message").value'),
    "mobile draft survives both swipe directions",
  );
  browser("set", "viewport", "320", "568", "2");
  evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
  browser("click", ".chat-title-menu > summary");
  browser("click", ".chat-header-menu button[aria-expanded]");
  drawerSettled();
  assert.equal(
    evaluate("document.documentElement.scrollWidth <= innerWidth"),
    true,
    "The header and Members must fit a small phone",
  );
  screenshot("mobile-small-members");
  browser("click", ".member-list-close");
  drawerGone();

  // A refused new DM stays in its dialog, without moving controls or the shell.
  const directArtifacts = process.env.NAVIGATION_TEST_DIRECT_ARTIFACTS;
  if (directArtifacts) mkdirSync(directArtifacts, { recursive: true });
  const directBounds = () =>
    evaluate(`['dialog[open]', '.start-direct-form input', '.start-direct-form .primary', '.direct-section'].map(selector => {
      const { x, y, width, height } = document.querySelector(selector).getBoundingClientRect();
      return { x, y, width, height };
    })`);
  for (const width of [1280, 390, 320]) {
    browser("set", "viewport", String(width), "844", "2");
    evaluate("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))");
    if (width <= 760 && !browsing()) {
      browser("click", ".navigation-toggle");
      settled();
    }
    browser("click", '[aria-label="New direct message"]');
    wait('document.querySelector(".start-direct-form input") === document.activeElement');
    browser("fill", ".start-direct-form input", "blocked_person");
    const before = directBounds();
    const captureDirect = (state) => {
      if (directArtifacts) browser("screenshot", "dialog[open]", `${directArtifacts}/dm-${width}-${state}.png`);
    };
    captureDirect("ready");
    for (const [code, message] of [
      ["dm_not_accepted", "This person isn’t accepting direct messages."],
      ["dm_blocked", "You blocked this person. Unblock them to message them."],
    ]) {
      evaluate(`navigationFixture.directError = ${JSON.stringify(code)}`);
      browser("click", ".start-direct-form .primary");
      wait('document.querySelector(".start-direct-form .primary").textContent === "Opening…"');
      assert.deepEqual(directBounds(), before, "Pending must not move the modal, field, button or sidebar");
      assert.equal(evaluate('document.querySelector(".start-direct-form input").readOnly'), true);
      assert.equal(evaluate('document.querySelector(".start-direct-error").textContent'), "");
      if (code === "dm_not_accepted") captureDirect("pending");
      evaluate("navigationFixture.directReleases.splice(0).forEach(release => release())");
      wait(`document.querySelector('.start-direct-error')?.textContent === ${JSON.stringify(message)}`);
      assert.deepEqual(directBounds(), before, "Rejection must not move the modal, field, button or sidebar");
      assert.equal(
        evaluate(`(() => {
          const sidebar = document.querySelector('.channel-navigation').cloneNode(true);
          sidebar.querySelectorAll('dialog').forEach(dialog => dialog.remove());
          return sidebar.textContent.includes('accepting direct messages');
        })()`),
        false,
      );
      assert.equal(evaluate('document.querySelector(".start-direct-form input").value'), "blocked_person");
      assert.equal(evaluate('document.querySelector(".start-direct-form .primary").disabled'), false);
      if (code === "dm_not_accepted") captureDirect("refused");
    }
    browser("fill", ".start-direct-form input", "another_person");
    assert.equal(evaluate('document.querySelector(".start-direct-error").textContent'), "");
    assert.deepEqual(directBounds(), before, "Editing must clear the error without collapsing its space");
    browser("press", "Escape");
    wait('!document.querySelector("dialog[open]")');
  }
  browser("set", "viewport", "390", "844", "2");
  if (process.env.NAVIGATION_TEST_NARROW_SCREENSHOT)
    browser("screenshot", process.env.NAVIGATION_TEST_NARROW_SCREENSHOT);
  console.log(
    "PASS: New-DM refusals stay inline; loading, refusal and editing preserve modal, input, button and sidebar bounds at 1280px, 390px and 320px.",
  );
  console.log(
    "PASS: desktop navigation/presence; Chromium touch: center-start Browse and Members swipes, header-button and backdrop swipes, finger tracking, fling/halfway/snap-back, direction/vertical/cancel guards, reduced motion, composer and overlays, release-click protection, draft/history retention, Back button, empty Pins and 320px layout.",
  );
  browser("open", url.toString());
  wait('document.querySelector("#chat-message") && !document.querySelector("#chat-message").disabled');
  assert.equal(evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded")'), "false");
  assert.equal(evaluate('document.querySelector(".space-member-presence")'), null);
  screenshot("mobile-members-default");
  console.log(
    "PASS: members start open on desktop, hide without losing the draft, and start closed on a fresh narrow visit.",
  );
} finally {
  socket?.close();
  try {
    browser("close");
  } catch {}
  rmSync(directory, { recursive: true, force: true });
}
