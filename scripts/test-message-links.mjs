// Real composer/link components with local-only delivery; no accounts or API writes.
// node scripts/test-message-links.mjs http://localhost:5173
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5173");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname));
const session = process.env.LINK_TEST_SESSION ?? `links-${process.pid}`;
const artifacts = process.env.LINK_TEST_ARTIFACTS && resolve(process.env.LINK_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, ...args, "--json"], { encoding: "utf8", timeout: 60000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(async () => { ${code} })()`).result;
const draft = "Join us 🙂 https://caper.chat/invite/AbCd1234, or www.Example.org/Notes.";
const draftLinks = ["https://caper.chat/invite/AbCd1234", "www.Example.org/Notes"];
const marks = () =>
  evaluate("return [...document.querySelectorAll('.chat-mention-input-mirror mark')].map(el => el.textContent);");
try {
  browser("open", origin.href);
  browser("set", "viewport", "1440", "1000", "2");
  browser("wait", '.experimental-note a[aria-label="Jose on X"]');
  const xStyle = () =>
    evaluate(`const style = getComputedStyle(document.querySelector('a[aria-label="Jose on X"]'));
    return [style.color, style.backgroundColor, style.boxShadow];`);
  const neutralX = ["rgb(243, 244, 245)", "rgba(0, 0, 0, 0)", "none"];
  assert.deepEqual(xStyle(), neutralX, "the X logo is neutral, not a highlighted URL");
  if (artifacts) browser("screenshot", ".experimental-note", `${artifacts}/homepage-contact.png`);
  browser("hover", 'a[aria-label="Jose on X"]');
  assert.deepEqual(xStyle(), neutralX, "the X logo stays neutral on hover");
  if (artifacts) browser("screenshot", ".experimental-note", `${artifacts}/homepage-contact-hover.png`);
  browser("set", "viewport", "1000", "640", "2");
  evaluate(`
    const source = await (await fetch('/src/chat/Composer.tsx')).text();
    const reactPath = source.match(/from "([^"]+react[.]js[^"]*)"/)[1];
    const {default: React} = await import(reactPath);
    const {default: {createRoot}} = await import(reactPath.replace('/react.js', '/react-dom_client.js'));
    const {default: Composer} = await import('/src/chat/Composer.tsx');
    const {default: LinkedText} = await import('/src/chat/LinkedText.tsx');
    await import('/src/chat/chat.css');
    for (const element of document.body.children) element.hidden = true;
    const mount = document.createElement('main'); mount.id = 'link-fixture';
    mount.style.cssText = 'padding:24px;max-width:760px;margin:40px auto;background:var(--surface-conversation);border-radius:8px';
    document.body.append(mount);
    window.sent = [];
    function Fixture() {
      const [messages, setMessages] = React.useState([]);
      return React.createElement(React.Fragment, null,
        React.createElement('p', {style: {color: 'var(--text-muted)', fontSize: 12}}, 'LOCAL TEST FIXTURE · Link highlighting'),
        React.createElement('h2', {style: {margin: '16px 0'}}, '# link-check'),
        React.createElement('section', {id: 'sent', style: {minHeight: 140}},
          messages.map((text, key) => React.createElement('p', {key, style: {padding: '16px 0', lineHeight: 1.8}}, React.createElement(LinkedText, {text})))),
        React.createElement('div', {className: 'chat-composer'}, React.createElement(Composer, {
          channelName: 'link-check', direct: false, disabled: false, identityReady: true, sending: false, sendRejected: false,
          onSend: async text => { window.sent.push(text); setMessages(previous => [...previous, text]); },
          onTyping: () => {}, onDraftPresence: () => {}, onResize: resize => { resize(); },
        })));
    }
    createRoot(mount).render(React.createElement(Fixture));
  `);
  browser("wait", "#link-fixture textarea");
  browser("fill", "#link-fixture textarea", draft);
  assert.deepEqual(marks(), draftLinks, "both invitation and ordinary draft URLs highlight");
  assert.equal(
    evaluate(`return [...document.querySelectorAll('.chat-draft-link')].every(mark => {
    const style = getComputedStyle(mark);
    return style.backgroundColor !== 'rgba(0, 0, 0, 0)' && style.borderRadius === '4px'
      && style.boxShadow !== 'none' && style.padding === '0px' && style.color === 'rgb(105, 207, 255)';
  });`),
    true,
    "draft URLs have baby-blue text and rounded backgrounds without moving glyphs",
  );
  assert.deepEqual(
    evaluate(`const style = getComputedStyle(document.querySelector('#link-fixture textarea'));
    return [style.color, style.caretColor];`),
    ["rgba(0, 0, 0, 0)", "rgb(243, 244, 245)"],
    "the native caret stays visible without duplicate foreground text",
  );
  browser("press", "Enter");
  browser("wait", "#sent a");
  assert.deepEqual(evaluate("return window.sent;"), [draft], "highlighting must not alter submitted text");
  assert.deepEqual(
    evaluate(
      "return [...document.querySelectorAll('#sent a')].map(a => [a.textContent, a.getAttribute('href'), a.target]);",
    ),
    [
      ["https://caper.chat/invite/AbCd1234", "https://caper.chat/invite/AbCd1234", "_blank"],
      ["www.Example.org/Notes", "https://www.Example.org/Notes", "_blank"],
    ],
  );
  assert.equal(evaluate("return document.querySelectorAll('#sent a svg[aria-hidden=true]').length;"), 2);
  assert.equal(evaluate("return document.querySelector('#sent').textContent;"), draft, "icons do not add copied text");
  assert.equal(evaluate("return getComputedStyle(document.querySelector('#sent a')).color;"), "rgb(105, 207, 255)");
  assert.equal(
    evaluate(`return [...document.querySelectorAll('#sent a')].every(a => {
      const style = getComputedStyle(a);
      return style.backgroundColor !== 'rgba(0, 0, 0, 0)' && style.boxShadow !== 'none'
        && style.borderRadius === '4px' && style.padding === '1px 3px'
        && style.textDecorationLine === 'none';
    });`),
    true,
    "both generic and invitation URLs have padded, rounded, glowing backgrounds at rest",
  );
  assert.equal(
    evaluate(`
    const event = new MouseEvent('contextmenu', {bubbles: true, cancelable: true});
    return document.querySelector('#sent a').dispatchEvent(event);
  `),
    true,
    "sent links retain the browser context menu",
  );
  browser("fill", "#link-fixture textarea", draft);
  if (artifacts) browser("screenshot", "#link-fixture", `${artifacts}/links-desktop.png`);
  if (evaluate("return matchMedia('(hover: hover)').matches;")) {
    const before = evaluate("return getComputedStyle(document.querySelector('#sent a')).backgroundColor;");
    browser("hover", "#sent a");
    assert.notEqual(evaluate("return getComputedStyle(document.querySelector('#sent a')).backgroundColor;"), before);
    if (artifacts) browser("screenshot", "#link-fixture", `${artifacts}/links-hover.png`);
  }
  browser("set", "viewport", "390", "844", "2");
  evaluate("await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));");
  assert.deepEqual(marks(), draftLinks);
  assert.equal(evaluate("return document.documentElement.scrollWidth <= innerWidth;"), true);
  assert.equal(
    evaluate(`
    return [...document.querySelectorAll('.chat-link-end')].every(tail => {
      const range = document.createRange(); range.selectNodeContents(tail.firstChild);
      const text = range.getBoundingClientRect(), icon = tail.querySelector('svg').getBoundingClientRect();
      return icon.left >= text.right && icon.top >= text.top && icon.bottom <= text.bottom;
    });
  `),
    true,
    "icons stay beside the final URL character on wrapped lines",
  );
  if (artifacts) browser("screenshot", `${artifacts}/links-narrow.png`);
  browser("fill", "#link-fixture textarea", "plain text, no link");
  assert.deepEqual(marks(), [], "removing a link clears its highlight");
  assert.equal(
    evaluate("return getComputedStyle(document.querySelector('#link-fixture textarea')).color;"),
    "rgb(243, 244, 245)",
    "plain drafts return to native text painting",
  );
  const mixed = "🙂 @feedback\nhttps://example.org/Notes";
  browser("fill", "#link-fixture textarea", mixed);
  browser("press", "Control+a");
  browser("press", "Control+c");
  browser("press", "Backspace");
  assert.deepEqual(marks(), []);
  browser("press", "Control+v");
  assert.equal(evaluate("return document.querySelector('#link-fixture textarea').value;"), mixed);
  assert.deepEqual(marks(), ["@feedback", "https://example.org/Notes"], "pasted links and mentions coexist");
  browser("press", "Home");
  browser("press", "Delete");
  assert.deepEqual(marks(), ["@feedback"], "editing away a URL prefix removes only its highlight");
  browser("press", "Control+z");
  assert.deepEqual(marks(), ["@feedback", "https://example.org/Notes"], "native undo restores the highlight");
  browser("fill", "#link-fixture textarea", draft);
  console.log(
    "PASS: neutral X logo, highlighted draft URLs, exact send payload, all-URL blue pills, decorative icons, context-menu allowance, and wrapping",
  );
} finally {
  if (!process.env.LINK_TEST_KEEP) browser("close");
}
