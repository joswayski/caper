// Disposable loopback fixture + Chromium keyboard checks, not native/device acceptance.
// Start native-parity-fixture.mjs and Vite before running this script.
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
    execFileSync("agent-browser", ["--session", "reactors-check", ...args, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const control = async (body) => {
  const response = await fetch(`${api}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.equal(response.status, 200);
};
const checkSelection = (index, focused = true) => {
  assert.deepEqual(
    evaluate(`(() => {
      const tabs = [...document.querySelectorAll('.chat-reactors [role="tab"]')];
      const panel = document.querySelector('.chat-reactors [role="tabpanel"]');
      const selected = tabs.find(tab => tab.getAttribute('aria-selected') === 'true');
      return {
        selected: tabs.indexOf(selected),
        tabbable: tabs.filter(tab => tab.tabIndex === 0).map(tab => tabs.indexOf(tab)),
        linked: tabs.every(tab => tab.getAttribute('aria-controls') === panel.id)
          && panel.getAttribute('aria-labelledby') === selected.id,
        focused: document.activeElement === selected,
      };
    })()`),
    { selected: index, tabbable: [index], linked: true, focused },
  );
};

try {
  assert.equal((await (await fetch(`${api}/health`)).json()).fixture, true);
  for (const [layout, width, height] of [
    ["desktop", 1280, 900],
    ["narrow", 390, 844],
  ]) {
    await control({ reset: true });
    const channelId = "chan00000001",
      messageId = "chan00000001m04";
    for (const [emoji, userId] of [
      ["🚀", "member000002"],
      ["👍", "member000001"],
      ["🎉", "owner0000001"],
    ])
      await control({ incomingReaction: { channelId, messageId, emoji, userId } });
    browser("open", "about:blank");
    browser("set", "viewport", String(width), String(height), "2");
    browser("cookies", "set", "caper_fixture", "owner", "--url", web, "--path", "/", "--sameSite", "Lax");
    browser("open", `${web}/spaces`);
    const row = '[data-message-key="00000000-0000-4000-8000-000000000004"]';
    wait(`!!document.querySelector('${row} .chat-reaction') && !document.querySelector('.chat-initial-messages')`);
    // Explicitly mocked transport failure: exercise the panel while loading and on retry.
    evaluate(`(() => {
      const original = window.fetch.bind(window);
      window.fetch = (input, init) => String(input).endsWith('/reactions') && (!init?.method || init.method === 'GET')
        ? new Promise(resolve => { window.releaseReactors = () => {
            window.fetch = original;
            resolve(Response.json({ error: 'TEST FIXTURE: unavailable' }, { status: 503 }));
          }; })
        : original(input, init);
    })()`);
    browser("focus", `${row} .chat-message-actions-trigger`);
    browser("press", "Enter");
    browser("find", "role", "button", "click", "--name", "View reactions", "--exact");
    wait('!!document.querySelector(".chat-reactors [role=status]") && !!window.releaseReactors');
    browser("focus", '.chat-reactors [aria-selected="true"]');
    checkSelection(0);
    browser("press", "End");
    checkSelection(2);
    evaluate("window.releaseReactors()");
    wait('!!document.querySelector(".chat-reactors [role=alert]")');
    checkSelection(2);
    browser("find", "role", "button", "click", "--name", "Retry", "--exact");
    wait('document.querySelector(".chat-reactors-list")?.textContent.includes("Fixture Owner")');
    browser("focus", '.chat-reactors [aria-selected="true"]');
    for (const [key, index, author] of [
      ["Home", 0, "Alex"],
      ["ArrowRight", 1, "Maya"],
      ["End", 2, "Fixture Owner"],
      ["ArrowRight", 0, "Alex"],
      ["ArrowLeft", 2, "Fixture Owner"],
      ["ArrowLeft", 1, "Maya"],
    ]) {
      browser("press", key);
      checkSelection(index);
      assert.equal(evaluate(`document.querySelector('.chat-reactors-list').textContent.includes('${author}')`), true);
    }
    browser("press", "Tab");
    assert.equal(evaluate('document.activeElement.getAttribute("role")'), "tabpanel");
    browser("press", "Shift+Tab");
    checkSelection(1);
    assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
    assert.equal(
      evaluate(`(() => { const rect = document.querySelector('.chat-reactors').getBoundingClientRect();
      return rect.left >= 0 && rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight; })()`),
      true,
    );
    if (artifacts) {
      evaluate(
        "document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))",
      );
      assert.equal(evaluate("devicePixelRatio"), 2);
      browser("screenshot", `${artifacts}/reactors-keyboard-${layout}.png`);
    }
    await control({ incomingReaction: { channelId, messageId, emoji: "👍", userId: "member000001", active: false } });
    wait('document.querySelectorAll(".chat-reactors [role=tab]").length === 2');
    assert.equal(evaluate(`document.querySelectorAll('.chat-reactors [role=tab][tabindex="0"]').length`), 1);
    assert.equal(
      evaluate('document.querySelector(".chat-reactors [role=tab][aria-selected=true]").getAttribute("aria-label")'),
      "🚀, 1",
    );
    browser("press", "Escape");
    wait('!document.querySelector(".chat-reactors")');
    browser("close");
  }
  console.log(
    "Reaction keyboard checks passed: desktop/narrow arrows, wrapping, Home/End, Tab order, ARIA links, loading/error/retry, live removal, Escape and viewport bounds.",
  );
} finally {
  browser("close");
}
