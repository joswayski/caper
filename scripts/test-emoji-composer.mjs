// Real composer against the disposable native-parity fixture, never live accounts.
// Start scripts/native-parity-fixture.mjs and Vite, then run with Vite's loopback URL.
// Owns/resets that fixture: do not share it with another running UI test.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5173");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname), "Loopback preview only");
const health = await fetch("http://127.0.0.1:3001/health").then((response) => response.json());
assert.equal(health.fixture, true, "The API must be the disposable fixture");
// Repeated runs must not push rows out of the virtualized message viewport.
const reset = await fetch("http://127.0.0.1:3001/__fixture/control", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ reset: true }),
});
assert.ok(reset.ok, "The disposable fixture must reset before testing");
const session = `emoji-${process.pid}`;
const artifacts = process.env.EMOJI_TEST_ARTIFACTS && resolve(process.env.EMOJI_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
function browser(...args) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, ...args, "--json"], { encoding: "utf8", timeout: 60_000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(() => { ${code} })()`).result;
const wait = (code) => browser("wait", "--fn", code);
const value = () => evaluate('return document.querySelector("#chat-message").value;');
const options = () => evaluate('return document.querySelectorAll("#chat-emoji-options [role=option]").length;');
const messages = () => evaluate('return document.querySelectorAll(".chat-message").length;');

try {
  browser("open", origin.href);
  browser("cookies", "set", "caper_fixture", "owner");
  browser("open", new URL("/spaces", origin).href);
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  assert.match(evaluate('return document.querySelector(".chat-messages").textContent;'), /TEST FIXTURE/);
  browser("set", "viewport", "1280", "800", "2");
  // Hold the catalog request to exercise loading without slowing other APIs.
  evaluate(
    'const fetch=window.fetch.bind(window); window.fetch=(input, options) => String(input).endsWith("/autocomplete-v3.json") ? new Promise(resolve => { window.releaseEmojiCatalog=() => fetch(input, options).then(resolve); }) : fetch(input, options);',
  );
  const beforeLoading = messages();
  browser("fill", "#chat-message", ":tom");
  wait('document.querySelector(".chat-emoji-suggestions")?.textContent.includes("Loading emoji")');
  browser("press", "Enter");
  assert.equal(value(), ":tom");
  assert.equal(messages(), beforeLoading, "Enter during loading must not send");
  if (artifacts) browser("screenshot", `${artifacts}/emoji-loading.png`);
  evaluate("releaseEmojiCatalog();");
  wait('!!document.querySelector("#chat-emoji-options")');
  for (const [width, height, layout] of [
    [1280, 800, "desktop"],
    [390, 844, "narrow"],
  ]) {
    browser("set", "viewport", String(width), String(height), "2");
    if (width < 760 && evaluate('return !!document.querySelector(".member-list-close");')) {
      browser("click", ".member-list-close");
      // The phone member list slides out before it is removed.
      wait('!document.querySelector(".space-member-presence")');
    }
    evaluate('document.querySelector("#chat-message").scrollIntoView({block:"nearest"});');
    const before = messages();
    browser("fill", "#chat-message", "@");
    wait('!!document.querySelector(".chat-mention-suggestions")');
    evaluate(
      "return document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));",
    );
    const mentionBounds = evaluate(
      'const popup=document.querySelector(".chat-mention-suggestions").getBoundingClientRect(); const composer=document.querySelector(".chat-composer form").getBoundingClientRect(); return {left:popup.left-composer.left,right:popup.right-composer.right,width:popup.width};',
    );
    assert.ok(
      Math.abs(mentionBounds.left) < 1 && Math.abs(mentionBounds.right) < 1,
      `Mention suggestions must span the composer at ${width}px: ${JSON.stringify(mentionBounds)}`,
    );
    if (artifacts) browser("screenshot", `${artifacts}/mentions-full-width-${layout}.png`);
    browser("press", "Escape");
    browser("fill", "#chat-message", "@ma");
    wait('!!document.querySelector("#chat-mention-maya")');
    browser("press", "Tab");
    assert.equal(value(), "@maya ");
    assert.equal(messages(), before, "Accepting a mention must not send");

    browser("fill", "#chat-message", ":");
    wait('document.querySelectorAll("#chat-emoji-options [role=option]").length === 6');
    const popupStyle = evaluate(
      'const popup=document.querySelector(".chat-emoji-suggestions"); return {width:popup.getBoundingClientRect().width,borderless:[...popup.querySelectorAll("button")].every(row => ["borderTopWidth","borderRightWidth","borderBottomWidth","borderLeftWidth"].every(side => getComputedStyle(row)[side] === "0px"))};',
    );
    assert.ok(popupStyle.width <= 260, `Popup must remain compact: ${popupStyle.width}px`);
    assert.ok(popupStyle.borderless, "Suggestion rows must not have individual borders");
    const labelGaps = evaluate(
      'return [...document.querySelectorAll("#chat-emoji-options button")].map(row => row.querySelector("span").getBoundingClientRect().left - row.querySelector("img").getBoundingClientRect().right);',
    );
    assert.ok(
      labelGaps.every((gap) => Math.abs(gap - 12) < 0.1),
      `Emoji/label gaps must be 12px: ${labelGaps}`,
    );
    assert.equal(
      evaluate('return document.querySelector("#chat-emoji-1f44d span").textContent;'),
      ":thumbs-up:",
      "Merged dash-separated names must remain visible",
    );
    wait(
      '[...document.querySelectorAll(".chat-emoji-suggestions img")].every(image => image.complete && image.naturalWidth > 0)',
    );
    if (artifacts) browser("screenshot", `${artifacts}/emoji-flat-${layout}.png`);
    browser("press", "ArrowDown");
    browser("press", "Tab");
    assert.equal(value(), "😀");
    assert.equal(messages(), before, "Accepting a suggestion must not send");
    assert.equal(evaluate("return document.activeElement.id;"), "chat-message");

    browser("fill", "#chat-message", "👩‍💻 hi :rocket suffix 🚀");
    evaluate(
      'document.querySelector("#chat-message").setSelectionRange("👩‍💻 hi :rocket".length, "👩‍💻 hi :rocket".length);',
    );
    // Exercise real caret events after positioning inside the draft.
    browser("press", "ArrowLeft");
    browser("press", "ArrowRight");
    wait('!!document.querySelector("#chat-emoji-1f680")');
    browser("click", "#chat-emoji-1f680");
    assert.equal(value(), "👩‍💻 hi 🚀 suffix 🚀");
    // The caret is restored on the next animation frame, after React commits the value.
    wait(`document.querySelector("#chat-message").selectionStart === ${"👩‍💻 hi 🚀".length}`);
    assert.equal(messages(), before);

    for (const [query, id, emoji, name] of [
      ["israel", "1f1ee-1f1f1", "🇮🇱", "israel"],
      ["united_states", "1f1fa-1f1f8", "🇺🇸", "united-states"],
      ["cote-divoire", "1f1e8-1f1ee", "🇨🇮", "cote-divoire"],
    ]) {
      browser("fill", "#chat-message", `:${query}`);
      wait(`!!document.querySelector("#chat-emoji-${id}")`);
      assert.equal(evaluate(`return document.querySelector("#chat-emoji-${id} span").textContent;`), `:${name}:`);
      assert.equal(
        evaluate(`return document.querySelector("#chat-emoji-${id}").getAttribute("aria-label");`),
        `Insert ${name} emoji`,
      );
      wait(
        '[...document.querySelectorAll(".chat-emoji-suggestions img")].every(image => image.complete && image.naturalWidth > 0)',
      );
      if (artifacts && query === "israel") browser("screenshot", `${artifacts}/emoji-flag-${layout}.png`);
      browser("press", "Enter");
      assert.equal(value(), emoji, "Country name inserts the original Unicode flag");
      assert.equal(messages(), before, "Selecting a flag must not send");
    }

    browser("fill", "#chat-message", ":face_with_open_eyes");
    wait('!!document.querySelector("#chat-emoji-1fae2")');
    assert.match(
      evaluate('return document.querySelector("#chat-emoji-1fae2").getAttribute("aria-label");'),
      /face-with-open-eyes-and-hand-over-mouth/,
    );
    assert.ok(
      evaluate(
        'const label=document.querySelector("#chat-emoji-1fae2 span"); return label.scrollWidth > label.clientWidth && getComputedStyle(label).textOverflow === "ellipsis";',
      ),
      "Long labels must truncate without widening the popup",
    );
    if (artifacts) browser("screenshot", `${artifacts}/emoji-long-${layout}.png`);

    for (const text of ["https://tom", "12:30", "word:tom", ":tom:"]) {
      browser("fill", "#chat-message", text);
      assert.equal(options(), 0, text);
    }
    browser("fill", "#chat-message", ":notanemojiname");
    wait('document.querySelector(".chat-emoji-suggestions")?.textContent.includes("No emoji found")');
    if (artifacts && layout === "narrow") browser("screenshot", `${artifacts}/emoji-empty.png`);
    browser("fill", "#chat-message", ":rocket");
    wait('!!document.querySelector("#chat-emoji-options")');
    evaluate(
      'document.querySelector("#chat-message").dispatchEvent(new CompositionEvent("compositionstart", {bubbles:true}));',
    );
    wait('!document.querySelector("#chat-emoji-options")');
    evaluate(
      'document.querySelector("#chat-message").dispatchEvent(new CompositionEvent("compositionend", {bubbles:true}));',
    );
    wait('!!document.querySelector("#chat-emoji-options")');
    browser("fill", "#chat-message", ":thumbs_up");
    wait('!!document.querySelector("#chat-emoji-1f44d")');
    browser("press", "Escape");
    assert.equal(options(), 0);
    assert.equal(value(), ":thumbs_up");
    browser("fill", "#chat-message", ":tomato");
    wait('!!document.querySelector("#chat-emoji-1f345")');
    const bounds = evaluate(
      'const r=document.querySelector(".chat-emoji-suggestions").getBoundingClientRect(); return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:innerWidth,height:innerHeight};',
    );
    assert.ok(
      bounds.left >= 0 && bounds.right <= bounds.width && bounds.top >= 0 && bounds.bottom <= bounds.height,
      JSON.stringify(bounds),
    );
    wait(
      '[...document.querySelectorAll(".chat-emoji-suggestions img")].every(image => image.complete && image.naturalWidth > 0)',
    );
    if (artifacts) browser("screenshot", `${artifacts}/emoji-${layout}.png`);
    browser("press", "Enter");
    assert.equal(value(), "🍅");
    assert.equal(messages(), before, "Enter inserts rather than sends");
    browser("press", "Enter");
    wait(
      `document.querySelectorAll(".chat-message").length === ${before + 1} && document.querySelector("#chat-message").value === ""`,
    );
    assert.equal(evaluate('return [...document.querySelectorAll(".chat-message p")].at(-1).textContent;'), "🍅");
    console.log(
      `PASS: ${layout} colon/search, arrows/Tab, pointer/caret/Unicode, Escape, URL/time boundaries, insertion then send, artwork and viewport bounds`,
    );
  }

  // The mobile drawer is one dialog, not three pickers layered on one another.
  // Only KLIPY is mocked; GIF/sticker sends still round-trip through the fixture API.
  const openPicker = () => browser("click", 'button[aria-label="Emojis, GIFs and stickers"]');
  browser("fill", "#chat-message", "Keep this draft");
  const beforePicker = messages();
  openPicker();
  wait('!!document.querySelector("[role=tabpanel]")');
  assert.equal(evaluate('return document.querySelectorAll(".chat-gif-tool").length;'), 0);
  assert.deepEqual(evaluate('return [...document.querySelectorAll("[role=tab]")].map(tab => tab.textContent);'), [
    "Emojis",
    "GIFs",
    "Stickers",
  ]);
  assert.equal(evaluate('return document.querySelectorAll("[role=dialog]").length;'), 1);
  browser("fill", 'input[aria-label="Search emoji"]', "tomato");
  wait(`!!document.querySelector('[data-unified="1f345"]')`);
  browser("click", '[data-unified="1f345"]');
  assert.equal(value(), "Keep this draft🍅");
  assert.equal(messages(), beforePicker, "Choosing an emoji inserts without sending");
  wait('!document.querySelector("[role=dialog]")');
  assert.equal(evaluate("return document.activeElement.id;"), "chat-message");

  for (const [path, kind, label] of [
    ["gifs", "gif", "GIFs"],
    ["stickers", "sticker", "Stickers"],
  ]) {
    const title = `TEST FIXTURE ${kind}`;
    const format = { url: `https://static.klipy.com/caper-fixture-${kind}.gif`, width: 120, height: 80 };
    evaluate(`
      const fetch = window.fetch.bind(window);
      window.fetch = (input, options) => String(input).includes('/fixtureKlipyKey/${path}/')
        ? Promise.resolve(Response.json(${JSON.stringify({
          data: {
            data: [{ slug: `fixture-${kind}`, title, file: { sm: { gif: format }, md: { gif: format } } }],
            has_next: false,
          },
        })})) : fetch(input, options);
    `);
    browser("network", "route", format.url, "--abort");
    openPicker();
    browser("find", "role", "tab", "click", "--name", label, "--exact");
    wait(`!!document.querySelector('.chat-gif-grid[data-kind="${kind}"] .chat-gif-tile')`);
    assert.equal(
      evaluate('return document.querySelectorAll("[role=tab]").length;'),
      3,
      "No nested GIF/sticker tab bar",
    );
    assert.equal(evaluate('return document.querySelectorAll("[role=dialog]").length;'), 1);
    browser("click", ".chat-gif-tile");
    wait(`!!document.querySelector('.chat-message-gif[data-kind="${kind}"][title="${title}"]')`);
    assert.equal(value(), "Keep this draft🍅", "Sending media must not clear the text draft");
    assert.equal(evaluate('return document.querySelectorAll("[role=dialog]").length;'), 0);
  }
  openPicker();
  browser("find", "role", "tab", "click", "--name", "Emojis", "--exact");
  browser("press", "End");
  assert.equal(evaluate("return document.activeElement.textContent;"), "Stickers");
  browser("press", "ArrowRight");
  assert.equal(evaluate("return document.activeElement.textContent;"), "Emojis");
  browser("press", "Escape");
  wait('!document.querySelector("[role=dialog]")');
  assert.equal(value(), "Keep this draft🍅");
  assert.equal(evaluate("return document.activeElement.id;"), "chat-message");
  browser("network", "unroute");
  console.log(
    "PASS: mobile single drawer, three tabs, emoji insertion, GIF/sticker send with draft preserved, tab keyboard navigation and Escape focus",
  );

  browser("network", "route", "**/emoji/twemoji-15/autocomplete-v3.json", "--abort");
  browser("reload");
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  browser("fill", "#chat-message", ":tom");
  wait('document.querySelector(".chat-emoji-suggestions")?.textContent.includes("unavailable")');
  assert.equal(value(), ":tom", "Catalog failure must retain the draft");
  if (artifacts) browser("screenshot", `${artifacts}/emoji-error.png`);
  browser("network", "unroute");
  browser("press", "Escape");
  browser("fill", "#chat-message", ":tomato");
  wait('!!document.querySelector("#chat-emoji-1f345")');
  console.log("PASS: controlled loading, simulated DOM composition, failed catalog recovery and draft preservation");
} catch (error) {
  console.error(browser("snapshot"));
  console.error(browser("errors"));
  throw error;
} finally {
  browser("close");
}
