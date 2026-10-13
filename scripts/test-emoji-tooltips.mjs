// Real composer and pointer/touch input in Chromium; no accounts, messages or SFU.
// Start Vite, then: node scripts/test-emoji-tooltips.mjs http://localhost:5174
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname), "Disposable local UI only");
const session = `emoji-${process.pid}`;
const artifacts = process.env.EMOJI_TEST_ARTIFACTS && resolve(process.env.EMOJI_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, ...args, "--json"], { encoding: "utf8", timeout: 60000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(async () => { ${code} })()`).result;
const wait = (ms) => new Promise((done) => setTimeout(done, ms));
let socket;
let command;
const value = () => evaluate("return document.querySelector('#emoji-fixture textarea').value;");
const tooltip = () => evaluate("return document.querySelector('.chat-emoji-tooltip')?.textContent ?? null;");
const point = (emoji) =>
  evaluate(`
  const span = [...document.querySelectorAll('#emoji-fixture [data-emoji]')].find(el => el.textContent === ${JSON.stringify(emoji)});
  const r = span.getBoundingClientRect(); return {x: r.x + r.width / 2, y: r.y + r.height / 2};
`);
const restore = (text) => {
  browser("fill", "#emoji-fixture textarea", text);
  assert.equal(value(), text);
};

try {
  browser("open", origin.href);
  browser("set", "viewport", "1280", "900", "2");
  evaluate(`
    const {default: React} = await import('/node_modules/.vite/deps/react.js');
    const {default: {createRoot}} = await import('/node_modules/.vite/deps/react-dom_client.js');
    const {default: Composer} = await import('/src/chat/Composer.tsx');
    await import('/src/chat/chat.css');
    await import('/src/motion.css');
    for (const element of document.body.children) element.hidden = true;
    const mount = document.createElement('main'); mount.id = 'emoji-fixture';
    mount.style.cssText = 'padding:72px 24px 24px;max-width:760px;margin:80px auto;background:var(--surface-conversation);border-radius:8px';
    document.body.append(mount);
    window.sent = [];
    createRoot(mount).render(React.createElement(React.Fragment, null,
      React.createElement('p', {style: {color: 'var(--text-muted)', fontSize: '12px'}}, 'LOCAL TEST FIXTURE · Emoji names in the message composer'),
      React.createElement('div', {className: 'chat-composer'}, React.createElement(Composer, {
        ref: handle => { window.composer = handle; }, channelName: 'tooltip-fixture', direct: false,
        disabled: false, identityReady: true, sending: false, sendRejected: false,
        onSend: async text => { window.sent.push(text); }, onTyping: () => {}, onDraftPresence: () => {},
        onResize: resize => { resize(); },
      }))));
  `);
  browser("wait", "#emoji-fixture textarea");
  const endpoint = browser("get", "cdp-url").cdpUrl;
  const targets = await (await fetch(`http://${new URL(endpoint).host}/json/list`)).json();
  const target = targets.find((item) => item.type === "page" && item.url.startsWith(origin.origin));
  assert.ok(target);
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, reject) => {
    socket.onopen = done;
    socket.onerror = reject;
  });
  let id = 0;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const result = JSON.parse(data);
    const request = pending.get(result.id);
    if (!request) return;
    pending.delete(result.id);
    if (result.error) request.reject(new Error(result.error.message));
    else request.resolve(result.result);
  };
  command = (method, params) =>
    new Promise((resolve, reject) => {
      const request = ++id;
      pending.set(request, { resolve, reject });
      socket.send(JSON.stringify({ id: request, method, params }));
    });
  const mouse = (p) => command("Input.dispatchMouseEvent", { type: "mouseMoved", ...p });
  const touch = (type, p) => command("Input.dispatchTouchEvent", { type, touchPoints: p ? [{ ...p, id: 1 }] : [] });

  restore("🎎");
  await mouse(point("🎎"));
  browser("wait", ".chat-emoji-tooltip");
  assert.equal(tooltip(), ":Japanese-dolls:");
  assert.equal(value(), "🎎");
  assert.equal(
    evaluate(
      "return document.querySelector('#emoji-fixture textarea').getAttribute('aria-describedby') === 'chat-composer-hint ' + document.querySelector('.chat-emoji-tooltip').id;",
    ),
    true,
  );
  if (artifacts) browser("screenshot", "#emoji-fixture", `${artifacts}/emoji-hover-desktop.png`);
  await mouse({ x: 1000, y: 60 });
  assert.equal(tooltip(), null);

  restore("before 👩‍💻 🇺🇸 ❤️ 1️⃣ after");
  for (const [emoji, label] of [
    ["👩‍💻", ":woman-technologist:"],
    ["🇺🇸", ":united-states:"],
    ["❤️", ":red-heart:"],
    ["1️⃣", ":keycap:-1:"],
  ]) {
    await mouse(point(emoji));
    browser("wait", ".chat-emoji-tooltip");
    assert.equal(tooltip(), label);
  }
  browser("press", "Escape");
  assert.equal(tooltip(), null);
  assert.equal(value(), "before 👩‍💻 🇺🇸 ❤️ 1️⃣ after");
  restore("prefix\n" + "wrapped text ".repeat(15) + "👩‍💻 suffix");
  await mouse(point("👩‍💻"));
  browser("wait", ".chat-emoji-tooltip");
  assert.equal(tooltip(), ":woman-technologist:", "hit targets follow wrapped lines");
  restore("plain text");
  assert.equal(tooltip(), null, "editing dismisses inspection");
  browser("set", "viewport", "390", "844", "2");
  await command("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
  assert.equal(evaluate("return matchMedia('(pointer: coarse)').matches;"), true);
  restore("🎎");
  const p = point("🎎");
  await touch("touchStart", p);
  await wait(650);
  assert.equal(tooltip(), ":Japanese-dolls:");
  if (artifacts) browser("screenshot", "#emoji-fixture", `${artifacts}/emoji-hold-mobile.png`);
  await touch("touchEnd");
  await wait(1650);
  assert.equal(tooltip(), null);
  await touch("touchStart", p);
  await wait(100);
  await touch("touchEnd");
  await wait(550);
  assert.equal(tooltip(), null, "a tap only edits");
  await touch("touchStart", p);
  await touch("touchMove", { x: p.x + 30, y: p.y + 20 });
  await wait(650);
  assert.equal(tooltip(), null, "moving cancels inspection");
  await touch("touchEnd");
  assert.equal(value(), "🎎");
  assert.deepEqual(evaluate("return window.sent;"), [], "inspection never sends a message");
  assert.deepEqual(browser("errors").errors, []);
  console.log(
    "PASS: emoji names on desktop hover and real coarse-pointer touch holds; joined emoji/flags/hearts/keycaps; Escape, tap, movement and release dismissal; draft and sends unchanged.",
  );
} finally {
  socket?.close();
  browser("close");
}
