// Browser regression against the loopback-only native parity fixture. No real accounts or writes.
// Start scripts/native-parity-fixture.mjs and the web dev server first.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const web = process.env.SIDEBAR_TEST_WEB_URL ?? "http://127.0.0.1:5174";
const api = process.env.SIDEBAR_TEST_API_URL ?? "http://127.0.0.1:3001";
for (const url of [web, api]) assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.SIDEBAR_TEST_ARTIFACTS && resolve(process.env.SIDEBAR_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), "caper-sidebar-"));
const init = join(scratch, "fixture.js");
function fixture() {
  if (location.protocol === "about:") return;
  const original = window.fetch.bind(window);
  const count = Number(new URL(location.href).searchParams.get("fixtureChannels") ?? 80);
  window.fetch = async (...args) => {
    const response = await original(...args);
    const path = new URL(typeof args[0] === "string" ? args[0] : args[0].url, location.href).pathname;
    if (!response.ok) return response;
    if (path === "/api/spaces" || path === "/api/spaces/space0000001") {
      const body = await response.json();
      const space = body.space ?? body.spaces[0];
      space.name = `TEST FIXTURE · ${count} channels`;
      if (body.channels) {
        body.channels = [
          body.channels[0],
          ...Array.from({ length: count - 1 }, (_, index) => ({
            id: `fixturech${String(index).padStart(3, "0")}`,
            spaceId: space.id,
            name:
              ["announcements", "introductions", "design", "development", "music", "gaming", "photography", "feedback"][
                index % 8
              ] + `-${index + 1}`,
            private: false,
            joined: true,
          })),
        ];
      }
      return Response.json(body);
    }
    if (path === "/api/dms")
      return Response.json({
        conversations: Array.from({ length: 40 }, (_, index) => ({
          id: `fixturedm${String(index).padStart(3, "0")}`,
          peer: {
            id: `fixtureusr${String(index).padStart(3, "0")}`,
            username: `fixture_${index}`,
            displayName: `Fixture person ${index + 1}`,
            avatarId: index * 17,
          },
          status: "accepted",
          lastSeq: "0",
          readSeq: "0",
        })),
      });
    return response;
  };
}
writeFileSync(init, `(${fixture.toString()})()`);
const args = ["--session", "sidebar-split-test", "--init-script", init];
if (process.env.SIDEBAR_TEST_CHROME) args.push("--executable-path", process.env.SIDEBAR_TEST_CHROME);
function browser(...command) {
  const result = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60_000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const handle = ".sidebar-sections-resize";
function geometry() {
  return evaluate(`(() => {
    const box = selector => { const r = document.querySelector(selector).getBoundingClientRect(); return { top: r.top, bottom: r.bottom, height: r.height, x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; };
    return { channels: box('#sidebar-channels'), directs: box('#sidebar-directs'), handle: box('${handle}'), header: box('.channel-navigation > header'), account: box('.voice-panel') };
  })()`);
}
function bounded() {
  const bounds = geometry();
  assert.ok(bounds.channels.height >= 60 && bounds.directs.height >= 60, JSON.stringify(bounds));
  assert.ok(bounds.channels.top >= bounds.header.bottom - 1);
  assert.ok(bounds.directs.bottom <= bounds.account.top + 1);
  assert.ok(evaluate("document.documentElement.scrollWidth <= innerWidth"));
  return bounds;
}
function reachableMenu(selector) {
  wait(`document.querySelector(${JSON.stringify(selector)})?.matches(':popover-open')`);
  assert.equal(
    evaluate(`(() => {
      const buttons = document.querySelector(${JSON.stringify(selector)}).querySelectorAll('button');
      const button = buttons[buttons.length - 1];
      const r = button.getBoundingClientRect();
      return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)?.closest('button') === button;
    })()`),
    true,
    "Menu actions remain reachable even when their sidebar pane is at its minimum height",
  );
}
function screenshot(name) {
  if (!artifacts) return;
  evaluate("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  browser("screenshot", `${artifacts}/${name}.png`);
}
try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  browser("open", "about:blank");
  browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
  browser("set", "viewport", "1440", "900", "2");
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
  wait(
    'document.querySelectorAll("#space-channel-list > li").length === 80 && document.querySelectorAll(".direct-line").length === 40',
  );
  evaluate("localStorage.removeItem('caper:sidebar-channels-share')");
  browser("dblclick", handle);
  const initial = bounded();
  assert.ok(Math.abs(initial.channels.height - initial.directs.height) < 1);
  assert.equal(evaluate(`getComputedStyle(document.querySelector('${handle}')).cursor`), "ns-resize");
  assert.equal(evaluate(`document.querySelector('${handle}').matches(':focus-visible')`), false);
  screenshot("sidebar-many-default");
  // Each scrollport moves without scrolling the other list or the outer page.
  browser("scroll", "down", "600", "--selector", "#sidebar-channels");
  wait('document.querySelector("#sidebar-channels").scrollTop > 0');
  assert.equal(evaluate('document.querySelector("#sidebar-directs").scrollTop'), 0);
  browser("scroll", "down", "600", "--selector", "#sidebar-directs");
  wait('document.querySelector("#sidebar-directs").scrollTop > 0');
  bounded();
  evaluate('document.querySelectorAll(".sidebar-section-scroll").forEach(node => node.scrollTop = 0)');
  // Pointer capture, persistence only on release, and the active resize cursor.
  const saved = evaluate("localStorage.getItem('caper:sidebar-channels-share')");
  browser("mouse", "move", String(initial.handle.x), String(initial.handle.y));
  browser("mouse", "down", "left");
  browser("mouse", "move", String(initial.handle.x), String(initial.handle.y - 100));
  assert.equal(evaluate("localStorage.getItem('caper:sidebar-channels-share')"), saved);
  assert.equal(evaluate("getComputedStyle(document.elementFromPoint(100, 150)).cursor"), "ns-resize");
  browser("mouse", "up", "left");
  const resized = bounded();
  assert.ok(Math.abs(resized.channels.height - (initial.channels.height - 100)) < 2);
  assert.equal(evaluate("document.documentElement.style.cursor"), "");
  screenshot("sidebar-more-dms");
  browser("reload");
  wait('document.querySelectorAll(".direct-line").length === 40');
  assert.ok(Math.abs(bounded().channels.height - resized.channels.height) < 2);
  browser("focus", handle);
  browser("press", "Home");
  assert.ok(bounded().channels.height >= 95);
  browser("hover", "#sidebar-channels .channel-section-heading");
  browser("click", ".channel-section-menu > summary");
  reachableMenu(".channel-section-menu .space-actions");
  browser("press", "Escape");
  browser("hover", "#space-channel-list > li:first-child");
  browser("click", '[aria-label="Manage general"]');
  browser("click", ".channel-menu[open] .notification-actions > button:first-child");
  reachableMenu(".channel-menu[open] .space-actions");
  screenshot("sidebar-minimum-menu");
  browser("scroll", "down", "80", "--selector", "#sidebar-channels");
  wait('!document.querySelector(".space-actions:popover-open")');
  evaluate('document.querySelector("#sidebar-channels").scrollTop = 0');
  browser("press", "Escape");
  browser("focus", handle);
  browser("press", "End");
  assert.ok(bounded().directs.height >= 95);
  browser("hover", '#sidebar-directs .direct-line:has([aria-label="Options for Fixture person 1"])');
  browser("click", '[aria-label="Options for Fixture person 1"]');
  reachableMenu(".direct-menu[open] .space-actions");
  browser("press", "Escape");
  browser("dblclick", handle);
  browser("focus", handle);
  browser("press", "ArrowDown");
  assert.ok(Math.abs(bounded().channels.height - initial.channels.height - 10) < 2);
  browser("dblclick", handle);
  browser("click", ".channel-section-toggle");
  assert.equal(evaluate('document.querySelector(".channel-section-toggle").getAttribute("aria-expanded")'), "false");
  bounded();
  browser("click", ".channel-section-toggle");
  // Sparse lists still retain an adjustable boundary.
  browser("open", `${web}/spaces?space=space0000001&channel=chan00000001&fixtureChannels=1`);
  wait('document.querySelectorAll("#space-channel-list > li").length === 1');
  bounded();
  screenshot("sidebar-one-channel");
  for (const [width, height] of [
    [1440, 540],
    [390, 844],
  ]) {
    browser("set", "viewport", String(width), String(height), "2");
    browser("open", `${web}/spaces?space=space0000001&channel=chan00000001`);
    wait('document.querySelectorAll("#space-channel-list > li").length === 80');
    wait(
      '!!document.querySelector(".chat-composer textarea:not(:disabled)") && !!document.querySelector(".channel-select[aria-current=page][aria-busy=false]")',
    );
    if (width < 761) {
      browser("click", ".navigation-toggle");
      wait('!!document.querySelector(".spaces-room.navigation-open:not([data-browse-slide])")');
    }
    bounded();
    screenshot(`sidebar-many-${width}x${height}`);
    browser("focus", handle);
    browser("press", "End");
    bounded();
    browser("press", "Home");
    bounded();
    browser("dblclick", handle);
  }
  assert.equal(evaluate('!!document.querySelector("vite-error-overlay")'), false);
  console.log(
    "PASS: 80 channels + 40 DMs; independent scrolling; drag/cursor; release-only persistence/reload; keyboard bounds/reset; reachable menus at minimum heights; collapsed/sparse channels; desktop, short-window and narrow layouts.",
  );
} finally {
  browser("close");
  rmSync(scratch, { recursive: true, force: true });
}
