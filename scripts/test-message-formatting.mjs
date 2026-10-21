// Real message components, local-only delivery. No account or API writes.
// node scripts/test-message-formatting.mjs http://localhost:5173
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5173");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname));
const session = process.env.FORMAT_TEST_SESSION ?? `format-${process.pid}`;
const artifacts = process.env.FORMAT_TEST_ARTIFACTS && resolve(process.env.FORMAT_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, "--restore", ...args, "--json"], {
      encoding: "utf8",
      timeout: 60000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(async () => { ${code} })()`).result;
const value = () => evaluate("return document.querySelector('#chat-message').value;");
const capture = (name) => {
  if (!artifacts) return;
  evaluate(`await Promise.all(document.getAnimations()
    .filter(animation => Number.isFinite(animation.effect.getComputedTiming().endTime))
    .map(animation => animation.finished.catch(() => {})));`);
  browser("screenshot", `${artifacts}/formatting-${name}.png`);
};
const clickToolbarEdge = (side) => {
  const point = evaluate(`
    const r = document.querySelector('.chat-format-toolbar').getBoundingClientRect();
    return {x: Math.round(${side === "left" ? "r.left + 1" : "r.right - 1"}), y: Math.round(r.top + r.height / 2)};
  `);
  browser("mouse", "move", String(point.x), String(point.y));
  browser("mouse", "down", "left");
  browser("mouse", "up", "left");
};
const select = (start, end) => {
  browser("click", "#chat-message");
  evaluate(
    `const field = document.querySelector('#chat-message'); field.focus(); field.setSelectionRange(${start}, ${end});`,
  );
  browser("press", "Shift");
};
try {
  browser("open", origin.href);
  browser("set", "viewport", "1000", "640", "2");
  browser("wait", ".experimental-note");
  evaluate(`
    const source = await (await fetch('/src/chat/Composer.tsx')).text();
    const reactPath = source.match(/from "([^"]+react[.]js[^"]*)"/)[1];
    const {default: React} = await import(reactPath);
    const {default: {createRoot}} = await import(reactPath.replace('/react.js', '/react-dom_client.js'));
    const {default: Composer} = await import('/src/chat/Composer.tsx');
    const {MessageText} = await import('/src/chat/MessageRow.tsx');
    const {default: MessageEditor} = await import('/src/chat/MessageEditor.tsx');
    await import('/src/chat/chat.css');
    for (const element of document.body.children) element.hidden = true;
    const mount = document.createElement('main'); mount.id = 'format-fixture';
    mount.style.cssText = 'padding:24px;max-width:760px;margin:40px auto;background:var(--surface-conversation);border-radius:8px';
    document.body.append(mount); window.sent = [];
    function Fixture() {
      const [messages, setMessages] = React.useState([]);
      const [editing, setEditing] = React.useState();
      const [pending, setPending] = React.useState();
      React.useEffect(() => { if (pending) { const timer = setTimeout(() => setPending(undefined), 100); return () => clearTimeout(timer); } }, [pending]);
      return React.createElement(React.Fragment, null,
        React.createElement('p', {style: {color: 'var(--text-muted)', fontSize: 12}}, 'LOCAL TEST FIXTURE · Bold & italic · Delivery simulated'),
        React.createElement('h2', {style: {margin: '16px 0'}}, '# formatting'),
        React.createElement('section', {id: 'sent', style: {minHeight: 180}}, messages.map((message, key) =>
          React.createElement('article', {key, style: {padding: '12px 0', lineHeight: 1.8}},
            React.createElement('p', null, React.createElement(MessageText, {message, mentionNames: new Map(), actions: {openMention: () => { window.mentionOpened = true; }}})),
            React.createElement('button', {onClick: () => setEditing(message), style: {fontSize: 12, color: 'var(--text-muted)'}}, 'Edit')))),
        React.createElement('div', {className: 'chat-composer'}, React.createElement(Composer, {
          channelName: 'formatting', direct: false, disabled: false, identityReady: true, sending: false, sendRejected: false,
          pendingSend: pending,
          onSend: async text => { window.sent.push(text); const id = crypto.randomUUID(); setMessages(previous => [...previous, {id, clientMessageId: id, revision: 1, content: {text, mentions: [{type:'feedback'}]}}]); setPending({clientMessageId: id, text}); },
          onTyping: () => {}, onDraftPresence: () => {}, onResize: resize => { resize(); },
        })),
        editing && React.createElement(MessageEditor, {message: editing, onClose: () => setEditing(undefined), onReload: async () => editing,
          onSave: async text => {setMessages(previous => previous.map(message => message.id === editing.id ? {...message, content: {...message.content, text}} : message)); window.edited = text; }}));
    }
    createRoot(mount).render(React.createElement(Fixture));
  `);
  browser("wait", "#chat-message");
  browser("fill", "#chat-message", "Make this bold and italic.");
  select(10, 14);
  browser("wait", ".chat-format-toolbar");
  assert.equal(
    evaluate(`
    const bar = document.querySelector('.chat-format-toolbar');
    const [bold, italic] = bar.querySelectorAll('button');
    const r = bar.getBoundingClientRect(), b = bold.getBoundingClientRect(), i = italic.getBoundingClientRect();
    return b.left === r.left && i.right === r.right && b.right === i.left &&
      b.height === r.height && i.height === r.height &&
      [bar, bold, italic].every(el => getComputedStyle(el).borderTopWidth === '0px');
  `),
    true,
    "borderless buttons fill the entire toolbar",
  );
  clickToolbarEdge("left");
  assert.equal(value(), "Make this **bold** and italic.");
  assert.equal(evaluate("return document.activeElement.id;"), "chat-message", "formatting retains editor focus");
  browser("press", "Control+z");
  assert.equal(value(), "Make this bold and italic.", "one undo removes formatting, not the draft");
  select(10, 14);
  browser("press", "Control+b");
  assert.equal(value(), "Make this **bold** and italic.");
  browser("press", "Control+b");
  assert.equal(value(), "Make this bold and italic.", "shortcut toggles off");
  browser("press", "Control+b");
  select(23, 29);
  clickToolbarEdge("right");
  assert.equal(value(), "Make this **bold** and _italic_.");
  browser("press", "ArrowRight");
  browser("press", "Enter");
  browser("wait", "#sent strong");
  assert.equal(evaluate("return document.querySelector('#sent strong').textContent;"), "bold");
  assert.equal(evaluate("return document.querySelector('#sent em').textContent;"), "italic");
  assert.equal(evaluate("return window.sent[0];"), "Make this **bold** and _italic_.");
  browser("fill", "#chat-message", "Both styles 🙂 with @feedback and https://example.com/a_b_c");
  select(0, 11);
  browser("press", "Control+b");
  browser("press", "Control+i");
  browser("press", "ArrowRight");
  browser("press", "Enter");
  browser("wait", "#sent strong em");
  assert.equal(evaluate("return document.querySelector('#sent strong em').textContent;"), "Both styles");
  assert.equal(evaluate("return document.querySelector('#sent a').getAttribute('href');"), "https://example.com/a_b_c");
  assert.equal(evaluate("return document.querySelector('#sent .chat-mention').textContent;"), "@feedback");
  browser("click", "#sent button");
  browser("wait", "#chat-edit-text");
  browser("click", "#chat-edit-text");
  browser("press", "Control+Home");
  browser("press", "Control+Shift+ArrowRight");
  browser("press", "Control+i");
  browser("focus", '.chat-format-toolbar button[aria-label="Bold"]');
  browser("press", "Tab");
  assert.equal(
    evaluate(`
    const button = document.activeElement, style = getComputedStyle(button);
    const bar = button.closest('.chat-format-toolbar');
    return button.getAttribute('aria-label') === 'Italic' && button.matches(':focus-visible') &&
      style.outlineStyle === 'none' && style.boxShadow === 'none' && style.borderTopWidth === '0px' &&
      style.borderRadius === '0px' && style.padding === '0px' &&
      button.getBoundingClientRect().height === bar.getBoundingClientRect().height;
  `),
    true,
    "edit dialog must not restore generic button borders or focus rings",
  );
  capture("edit");
  browser("click", ".chat-edit-save");
  browser("wait", "#chat-message");
  assert.equal(evaluate("return window.edited;"), "_Make_ this **bold** and _italic_.");
  assert.equal(evaluate("return document.querySelector('#sent em').textContent;"), "Make");
  browser("fill", "#chat-message", "Select these words to format them");
  select(7, 18);
  browser("wait", ".chat-format-toolbar");
  capture("desktop");
  browser("press", "Escape");
  assert.equal(evaluate("return document.querySelector('.chat-format-toolbar');"), null, "Escape dismisses formatting");
  browser("set", "viewport", "390", "844", "2");
  select(7, 18);
  browser("wait", ".chat-format-toolbar");
  assert.equal(evaluate("return document.documentElement.scrollWidth <= innerWidth;"), true);
  assert.equal(
    evaluate(
      "const r = document.querySelector('.chat-format-toolbar').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;",
    ),
    true,
  );
  capture("narrow");
  browser("fill", "#chat-message", "x".repeat(3997));
  browser("press", "Control+a");
  browser("wait", ".chat-format-toolbar");
  assert.equal(
    evaluate("return document.querySelector('.chat-format-toolbar button[aria-label=Bold]').disabled;"),
    true,
  );
  browser("press", "Control+b");
  assert.equal(value().length, 3997, "formatting cannot exceed the message limit");
  browser("fill", "#chat-message", "Select these words to format them");
  browser("set", "viewport", "1000", "640", "2");
  select(7, 18);
  browser("focus", '.chat-format-toolbar button[aria-label="Bold"]');
  browser("press", "Tab");
  assert.equal(
    evaluate(`
    const button = document.activeElement, style = getComputedStyle(button);
    return button.getAttribute('aria-label') === 'Italic' && button.matches(':focus-visible') &&
      style.outlineStyle === 'none' && style.boxShadow === 'none' &&
      style.backgroundColor !== getComputedStyle(button.parentElement).backgroundColor;
  `),
    true,
    "keyboard focus uses a background, never a ring",
  );
  capture("keyboard");
  browser("press", "Enter");
  assert.equal(value(), "Select _these words_ to format them", "keyboard-focused button remains actionable");
  select(8, 19);
  console.log(
    "PASS: toolbar, shortcuts, toggling, undo, send, nested emphasis, links, mentions, edit, dismissal, narrow layout and 4,000-character limit",
  );
} finally {
  if (!process.env.FORMAT_TEST_KEEP) browser("close");
}
