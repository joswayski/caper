// Real Chromium touch input against the disposable native-parity fixture, never production.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const web = process.env.NAVIGATION_TEST_WEB_URL ?? "http://localhost:5174";
const api = process.env.NAVIGATION_TEST_API_URL ?? "http://localhost:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.NAVIGATION_TEST_ARTIFACTS && resolve(process.env.NAVIGATION_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const browser = (...args) => {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", "release-check", ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const browsing = () => evaluate('document.querySelector(".spaces-room").classList.contains("navigation-open")');
const settled = () =>
  browser(
    "wait",
    "--fn",
    '!document.documentElement.dataset.browseTransition && !document.getAnimations().some(a => a.effect?.target?.matches?.(".space-member-presence"))',
  );
const capture = (name) => {
  if (artifacts) browser("screenshot", `${artifacts}/${name}.png`);
};

let socket;
let recording = false;
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  browser("open", "about:blank");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
  browser("set", "viewport", "390", "844", "2");
  browser("wait", "--fn", '!!document.querySelector(".chat-message") && !!document.startViewTransition');
  evaluate("document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))");
  browser("fill", "#chat-message", "Draft survives smooth navigation");
  browser("click", ".chat-heading");

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
  const target = targetInfos.find((target) => target.type === "page" && target.url.startsWith(web));
  assert.ok(target);
  const { sessionId } = await cdp("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  await cdp("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 2 }, sessionId);
  assert.equal(evaluate('matchMedia("(pointer: coarse)").matches'), true);
  assert.equal(evaluate("devicePixelRatio"), 2);
  let clock = 0;
  const touch = (type, x, y, after = 0.016) => {
    clock = type === "touchStart" ? Math.max(clock, Date.now() / 1000) : clock + after;
    return cdp(
      "Input.dispatchTouchEvent",
      {
        type,
        timestamp: clock,
        touchPoints: type === "touchEnd" ? [] : [{ x, y }],
      },
      sessionId,
    );
  };

  // Assert the rendered offset, not animation duration or internal easing parameters.
  // A held release must start gently, move monotonically and reach its destination.
  const checkRelease = async ({ members = false, fraction = 0.68, held = true, commit = true, captureName }) => {
    const open = members
      ? evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded") === "true"')
      : browsing();
    const direction = members ? (open ? 1 : -1) : open ? -1 : 1;
    const geometry = evaluate(`(() => {
      const room = document.querySelector('.spaces-room'), r = room.getBoundingClientRect();
      const region = document.querySelector(${JSON.stringify(members && open ? ".space-member-presence" : !members && open ? ".channel-navigation" : ".chat-scroller")}).getBoundingClientRect();
      return { x: ${direction} > 0 ? region.left + 4 : region.right - 4,
        y: region.bottom - 70,
        travel: ${members} ? Math.min(280, r.width - 64) + 8 + 48 : r.width + 48 };
    })()`);
    const dx = direction * geometry.travel * fraction;
    await touch("touchStart", geometry.x, geometry.y);
    const steps = held ? 40 : 8;
    for (let step = 1; step <= steps; step++) {
      await touch("touchMove", geometry.x + (dx * step) / steps, geometry.y, held ? 0.03 : 0.012);
      await pause(held ? 20 : 8);
    }
    // Let the last drag frame paint; hold longer than the velocity sampling window.
    if (held) await pause(150);
    const pseudo = `::view-transition-${open ? "new" : "old"}(browse-room)`;
    evaluate(`(() => {
      const offset = () => {
        const panel = document.querySelector('.space-member-presence');
        const style = ${members} ? (panel ? getComputedStyle(panel).transform : 'none')
          : getComputedStyle(document.documentElement, ${JSON.stringify(pseudo)}).transform;
        return new DOMMatrixReadOnly(style).m41;
      };
      window.releaseSamples = [];
      window.releaseOffset = offset();
      window.addEventListener('pointerup', () => {
        const start = performance.now();
        const sample = () => {
          const ms = performance.now() - start;
          window.releaseSamples.push({ ms, x: offset() });
          if (ms < 450 && (${members} || document.documentElement.dataset.browseTransition)) requestAnimationFrame(sample);
        };
        requestAnimationFrame(sample);
      }, { once: true });
    })()`);
    if (captureName) capture(captureName);
    await touch("touchEnd", 0, 0, held ? 0.2 : 0.012);
    await pause(500);
    settled();
    const result = evaluate("({ from: window.releaseOffset, frames: window.releaseSamples })");
    assert.ok(result.frames.length >= 3, "The release must paint intermediate positions");
    const to = members ? (open !== commit ? 0 : geometry.travel) : open !== commit ? geometry.travel : 0;
    const distance = Math.abs(to - result.from);
    assert.ok(distance > 10, "Exercise a visible remainder, not an already-complete gesture");
    const frames = result.frames.filter((frame) => frame.ms < (held ? 250 : 100));
    let previous = 0;
    for (const frame of frames) {
      const progress = (frame.x - result.from) / (to - result.from);
      assert.ok(progress >= previous - 0.02 && progress <= 1.02, "No reversal, jump to the wrong pose or overshoot");
      if (held && frame.ms <= 50)
        assert.ok(progress < 0.3, "A held release must not slam through most of its remainder in the first 50ms");
      previous = progress;
    }
    assert.equal(
      members
        ? evaluate('document.querySelector(".member-list-toggle").getAttribute("aria-expanded") === "true"')
        : browsing(),
      open !== commit,
      "Keep halfway/flick navigation rules",
    );
    console.log(
      `${members ? "Members" : "Browse"} ${open ? "close" : "open"}, ${held ? "held" : "fling"}, ${commit ? "commit" : "cancel"}: ${result.frames.length} painted frames; early positions ${frames
        .filter((f) => f.ms <= 50)
        .map((f) => `${Math.round(f.ms)}ms=${Math.round((Math.abs(f.x - result.from) / distance) * 100)}%`)
        .join(", ")}`,
    );
  };

  capture("swipe-conversation");
  if (artifacts) {
    browser("record", "start", `${artifacts}/browse-release.mp4`, "--fps", "30");
    recording = true;
  }
  await checkRelease({ captureName: "swipe-opening-held" });
  capture("swipe-browse");
  await checkRelease({ captureName: "swipe-closing-held" });
  if (recording) {
    recording = false;
    browser("record", "stop");
  }
  await checkRelease({ fraction: 0.3, commit: false });
  await checkRelease({ fraction: 0.2, held: false });
  await checkRelease({ fraction: 0.2, held: false });
  if (artifacts) {
    browser("record", "start", `${artifacts}/members-release.mp4`, "--fps", "30");
    recording = true;
  }
  await checkRelease({ members: true, fraction: 0.65, captureName: "members-opening-held" });
  capture("swipe-members");
  await checkRelease({ members: true, fraction: 0.65, captureName: "members-closing-held" });
  if (recording) {
    recording = false;
    browser("record", "stop");
  }
  assert.equal(evaluate('document.querySelector("#chat-message").value'), "Draft survives smooth navigation");
  browser("set", "media", "reduced-motion");
  browser("find", "role", "button", "click", "--name", "Back to Browse", "--exact");
  assert.equal(browsing(), true);
  assert.equal(evaluate('document.getAnimations().some(a => a.effect?.pseudoElement?.includes("browse-room"))'), false);
  console.log(
    "PASS: smooth bidirectional held releases, snap-back, flicks, member drawer, retained draft and reduced motion.",
  );
} finally {
  try {
    if (recording) browser("record", "stop");
  } finally {
    socket?.close();
    browser("close");
  }
}
