// Real homepage UI, local demo state only. No live auth or chat requests are sent.
// Run against local Vite: node scripts/test-home-demo.mjs http://localhost:30701
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const origin = new URL(process.argv[2] ?? "http://localhost:5174");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname), "Local development only");
const session = `home-${process.pid}`;
// Control the clock only in this browser fixture, without shipping demo controls.
const fixture = mkdtempSync(join(tmpdir(), "caper-demo-"));
const initScript = join(fixture, "timing.js");
writeFileSync(
  initScript,
  `
  Math.random = () => 0.5;
  let time = 1000;
  performance.now = () => time;
  const interval = window.setInterval.bind(window);
  const clear = window.clearInterval.bind(window);
  const clocks = new Set();
  window.demoClockActive = false;
  window.setInterval = (callback, delay, ...args) => {
    const id = interval(delay === 75 ? () => {} : callback, delay, ...args);
    if (delay === 75) {
      clocks.add(id);
      window.demoClockActive = true;
      window.advanceDemoClock = seconds => { time += seconds * 1000; callback(...args); };
    }
    return id;
  };
  window.clearInterval = id => {
    clocks.delete(id);
    window.demoClockActive = clocks.size > 0;
    clear(id);
  };
`,
);
function browser(...args) {
  const result = JSON.parse(
    execFileSync("agent-browser", ["--session", session, "--init-script", initScript, ...args, "--json"], {
      encoding: "utf8",
      timeout: 60000,
    }),
  );
  assert.ok(result.success, result.error);
  return result.data;
}
const evaluate = (code) => browser("eval", `(async () => { ${code} })()`).result;
const wait = (code) => browser("wait", "--fn", `Boolean(${code})`);
const nextMoments = (count) =>
  evaluate(
    `for (let i = 0; i < ${count}; i++) { window.advanceDemoClock(1); await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))); }`,
  );
const members = () => evaluate('return [...document.querySelectorAll(".sim-member strong")].map(el => el.textContent)');
const voices = () => evaluate('return [...document.querySelectorAll(".sim-person strong")].map(el => el.textContent)');
const speakers = () =>
  evaluate('return [...document.querySelectorAll(".sim-person[data-speaking] strong")].map(el => el.textContent)');
const checkVoiceSummary = (people) => {
  const summary = evaluate(`
    const stack = document.querySelector('.sim-sidebar .voice-stack');
    const count = stack.querySelector('.voice-stack-count');
    const frames = [...stack.querySelectorAll('.voice-stack-avatar'), ...document.querySelectorAll('.sim-voice-avatar')].map(el => ({
      style: getComputedStyle(el),
      speaking: el.classList.contains('speaking'),
    }));
    return {
      avatars: [...stack.querySelectorAll('.voice-stack-avatar')].filter(el => el.getClientRects().length > 0).map(el => el.querySelector('.sim-avatar').getAttribute('aria-label')),
      overflow: stack.querySelector('.voice-stack-faces small')?.textContent ?? '',
      label: count.textContent,
      accessibleLabel: stack.getAttribute('aria-label'),
      clipped: count.scrollWidth > count.clientWidth,
      transparentBackgrounds: frames.every(({ style }) => style.backgroundColor === 'rgba(0, 0, 0, 0)'),
      roundFrames: frames.every(({ style }) => style.borderRadius === '50%'),
      unframedIdleAvatars: frames.filter(({ speaking }) => !speaking).every(({ style }) => style.borderColor === 'rgba(0, 0, 0, 0)' && style.boxShadow === 'none'),
      speakingRings: frames.filter(({ speaking }) => speaking).every(({ style }) => style.borderColor === 'rgb(99, 122, 67)' && style.boxShadow !== 'none'),
    };
  `);
  assert.deepEqual(
    summary,
    {
      avatars: people.map((name) => `${name}'s caper avatar`),
      overflow: "",
      label: `${people.length} in voice`,
      accessibleLabel: `${people.length} in demo voice`,
      clipped: false,
      transparentBackgrounds: true,
      roundFrames: true,
      unframedIdleAvatars: true,
      speakingRings: true,
    },
    "Every voice avatar must be visible without a gray frame; round speaking rings and the full voice total remain",
  );
};
const pointFor = (selector) =>
  evaluate(`
    const element = document.querySelector(${JSON.stringify(selector)});
    const box = element.getBoundingClientRect();
    const viewport = element.closest('.sim-messages')?.getBoundingClientRect();
    const left = Math.max(box.left, 0), right = Math.min(box.right, innerWidth);
    const top = Math.max(box.top, viewport?.top ?? 0, 0), bottom = Math.min(box.bottom, viewport?.bottom ?? innerHeight, innerHeight);
    for (const across of [.1, .5, .9]) for (const down of [.25, .5, .75]) {
      const x = Math.round(left + (right - left) * across), y = Math.round(top + (bottom - top) * down);
      const hit = document.elementFromPoint(x, y);
      if (element.closest('.live-stage').contains(hit) && !hit.closest('[data-live-control]')) return [x, y];
    }
    throw new Error('Preview target is obscured: ' + ${JSON.stringify(selector)});
  `);
const checkReadOnlyHover = (selectors) => {
  for (const selector of selectors) {
    browser("mouse", "move", "0", "0");
    const appearance = () =>
      evaluate(`
      const style = getComputedStyle(document.querySelector(${JSON.stringify(selector)}));
      return [style.backgroundColor, style.borderColor, style.color, style.textDecorationLine];
    `);
    const before = appearance();
    const point = pointFor(selector);
    browser("mouse", "move", ...point.map(String));
    assert.equal(
      evaluate(`return document.querySelector(${JSON.stringify(selector)}).matches(':hover')`),
      false,
      `${selector} is decorative, not a hover target`,
    );
    assert.deepEqual(appearance(), before, `${selector} must not highlight on hover`);
    assert.equal(evaluate(`return getComputedStyle(document.elementFromPoint(${point.join(",")})).cursor`), "grab");
    assert.equal(
      evaluate('return getComputedStyle(document.querySelector(".live-invite")).backgroundColor'),
      "rgb(243, 244, 245)",
      "Hovering the illustration must not highlight the join button",
    );
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
    assert.equal(evaluate("return location.pathname"), "/", "Decorative content clicks never navigate");
  }
  browser("mouse", "move", "0", "0");
};
const drag = (selector) => {
  const point = pointFor(selector);
  browser("mouse", "move", ...point.map(String));
  browser("mouse", "down", "left");
  try {
    evaluate(
      'window.dragStartTilt = Number(document.querySelector(".live-scene").style.transform.match(/rotateY\\(([-\\d.]+)deg\\)/)[1]);',
    );
    browser("mouse", "move", String(point[0] + 40), String(point[1] + 20));
    assert.equal(
      evaluate('return document.querySelector(".live-stage").hasAttribute("data-dragging")'),
      true,
      `Dragging ${selector} tilts the preview`,
    );
    assert.equal(
      evaluate(`return getComputedStyle(document.querySelector(${JSON.stringify(selector)})).cursor`),
      "grabbing",
    );
    wait(
      'Number(document.querySelector(".live-scene").style.transform.match(/rotateY\\(([-\\d.]+)deg\\)/)[1]) > window.dragStartTilt + 1',
    );
  } finally {
    browser("mouse", "up", "left");
  }
  assert.equal(evaluate('return document.querySelector(".live-stage").hasAttribute("data-dragging")'), false);
  assert.equal(evaluate("return location.pathname"), "/", "Dragging never activates the join link");
  browser("mouse", "move", "0", "0");
};

try {
  browser("open", origin.href);
  browser("network", "route", "**/api/account/me", "--body", "null");
  browser("set", "viewport", "1280", "800", "2");
  browser("set", "media", "dark", "reduced-motion");
  browser("reload");
  wait('document.querySelector(".live-stage[data-ready]") && !window.demoClockActive');
  const before = evaluate('return document.querySelector(".sim-messages").textContent');
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-message:last-of-type .sim-reaction").length'),
    0,
    "A newly posted message starts without reactions",
  );
  browser("wait", "2500");
  assert.equal(
    evaluate('return document.querySelector(".sim-messages").textContent'),
    before,
    "Reduced motion freezes the script",
  );
  browser("set", "media", "dark");
  wait("window.demoClockActive");
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".live-activator")).placeItems'), "center");
  assert.equal(evaluate('return document.querySelector(".live-activator").getAttribute("href")'), "/spaces");

  assert.equal(
    evaluate('return document.querySelector(".experimental-warning").textContent'),
    "Caper is a work in progress and may contain bugs or incomplete features.",
  );
  assert.equal(
    evaluate('return getComputedStyle(document.querySelector(".experimental-note")).color'),
    "rgb(185, 188, 190)",
    "Contact copy stays neutral",
  );
  assert.notEqual(
    evaluate('return getComputedStyle(document.querySelector(".experimental-warning")).color'),
    "rgb(185, 188, 190)",
    "Only the work-in-progress sentence gets a warm tint",
  );

  // Production surfaces from shared/design.css; don't derive expectations from the demo.
  assert.deepEqual(
    evaluate(
      'return [".sim-sidebar", ".sim-chat", ".sim-composer"].map(s => getComputedStyle(document.querySelector(s)).backgroundColor)',
    ),
    ["rgb(21, 28, 30)", "rgb(25, 33, 35)", "rgb(40, 49, 51)"],
  );
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".sim-avatar")).borderRadius'), "30%");
  assert.deepEqual(
    evaluate(
      'return [...document.fonts].filter(font => font.family === "Satoshi").map(font => [font.weight, font.display]).sort()',
    ),
    [
      ["400", "swap"],
      ["500", "swap"],
      ["700", "swap"],
      ["900", "swap"],
    ],
    "All font weights must paint fallback text while Satoshi downloads",
  );
  assert.equal(
    evaluate('return document.querySelector(".sim-chat > header").textContent'),
    "# general",
    "No simulated-demo badge inside the room",
  );
  assert.equal(
    evaluate('return document.querySelector(".live-caption")'),
    null,
    "No visible fictional-conversation caption",
  );
  assert.equal(evaluate('return document.querySelector(".sim-account .account-name").textContent'), "Maya");
  assert.equal(
    evaluate(
      'return document.querySelectorAll(".sim-account .voice-icon-button svg, .sim-account > .call-settings-trigger svg").length',
    ),
    3,
  );
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-account .device-menu svg").length'),
    2,
    "Input and output options match the actual account dock",
  );
  assert.equal(
    evaluate('return document.querySelector(".sim-direct .direct-section-title").textContent'),
    "Direct messages",
  );
  assert.equal(
    evaluate('return document.querySelector(".sim-sidebar .voice-dock strong").textContent'),
    "Voice connected",
  );
  assert.equal(evaluate('return document.querySelector(".sim-channel .voice-session-timer").textContent'), "00:03");
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".sim-members")).width'), "220px");
  assert.equal(evaluate('return document.querySelectorAll(".sim-member .presence-dot[data-status=online]").length'), 3);
  assert.deepEqual(
    evaluate(
      'return [".sim-sidebar .channel-navigation > header", ".sim-chat > header"].map(s => document.querySelector(s).offsetHeight)',
    ),
    [54, 54],
    "Sidebar and chat header dividers align",
  );
  checkVoiceSummary(["Maya", "Theo", "June"]);
  const [accountBounds, composerBounds] = evaluate(`
    const geometry = selector => {
      let element = document.querySelector(selector), top = 0;
      const height = element.offsetHeight;
      while (element) { top += element.offsetTop + (element.offsetParent?.clientTop ?? 0); element = element.offsetParent; }
      return [top, height];
    };
    return [geometry('.sim-account'), geometry('.sim-composer')];
  `);
  assert.deepEqual(accountBounds, composerBounds, "Account controls and single-line composer align");
  assert.equal(
    evaluate(`
    const leftOf = selector => {
      let element = document.querySelector(selector), left = 0;
      while (element) { left += element.offsetLeft + (element.offsetParent?.clientLeft ?? 0); element = element.offsetParent; }
      return left;
    };
    return leftOf('.sim-direct .direct-select > span:nth-child(2)') - leftOf('.sim-account .account-name');
  `),
    0,
    "Simulation profile name aligns with the direct-message name above it, independent of the scene tilt",
  );
  assert.equal(
    evaluate('return getComputedStyle(document.querySelector(".sim-direct .direct-avatar > .sim-avatar")).transform'),
    "none",
    "Simulation avatar artwork is not shifted into its clipping edge",
  );
  assert.equal(
    evaluate('return getComputedStyle(document.querySelector(".sim-sidebar .channel-select")).cursor'),
    "grab",
  );
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".live-invite")).cursor'), "pointer");
  checkReadOnlyHover([
    ".sim-message:last-of-type",
    ".sim-reaction",
    ".sim-avatar",
    ".sim-person",
    ".sim-channel-list > li:last-child .channel-line",
    ".sim-channel-list > li:last-child .channel-select",
    ".channel-manage",
    ".channel-join",
    ".direct-select",
    ".direct-action",
    ".browse-channels",
    ".voice-dock-channel",
    ".voice-hangup",
    ".account-profile",
    ".voice-icon-button",
    ".call-settings-trigger",
    ".member-list-toggle",
  ]);
  browser("hover", ".live-invite");
  wait('getComputedStyle(document.querySelector(".live-invite")).backgroundColor === "rgb(182, 77, 50)"');
  browser("mouse", "move", "0", "0");
  browser("focus", ".sim-messages");
  browser("press", "Tab");
  assert.equal(
    evaluate('return document.activeElement.matches(".live-activator")'),
    true,
    "Tab skips decorative controls and reaches join after the scrollable history",
  );
  assert.notEqual(
    evaluate('return getComputedStyle(document.querySelector(".live-invite")).outlineStyle'),
    "none",
    "The real join link keeps keyboard focus feedback",
  );
  evaluate("document.activeElement.blur();");
  console.log(
    "PASS: decorative messages, reactions, profiles, invite, channel and voice controls never highlight or navigate; only join keeps hover and keyboard focus feedback",
  );
  for (const selector of [
    ".sim-brand",
    ".sim-members-heading",
    ".sim-chat > header h2",
    ".sim-message:last-of-type p",
    ".sim-meme",
    ".sim-reaction img",
    ".sim-composer",
  ])
    drag(selector);
  console.log(
    "PASS: both sidebars, chat header, message text, meme image and composer drag the preview without navigating",
  );
  assert.equal(evaluate('return document.querySelector(".sim-composer").tagName'), "DIV");
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-demo :is(button, a, input, textarea, [role=button])").length'),
    0,
    "Join is the only action; the room is read-only",
  );
  assert.equal(evaluate('return document.querySelectorAll(".live-stage a").length'), 1);
  assert.ok(
    !evaluate(
      'return /joined voice|joined the channel|Try a reaction|tomato-soup/.test(document.querySelector(".sim-demo").textContent)',
    ),
  );
  assert.deepEqual(
    evaluate(
      'return [...document.querySelectorAll(".sim-channel-list .channel-select > span:first-of-type")].map(el => el.textContent)',
    ),
    ["general", "feedback"],
  );
  assert.notEqual(evaluate('return getComputedStyle(document.querySelector(".sim-members")).display'), "none");
  assert.ok(
    evaluate(
      'return !!document.querySelector(".sim-people").closest("li[data-voice]")?.querySelector(".channel-select[aria-current=page]")',
    ),
    "Voice participants belong beneath their channel",
  );
  assert.ok(!evaluate('return document.querySelector(".sim-sidebar").textContent.includes("In voice")'));
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".sim-person .avatar")).width'), "20px");
  assert.equal(
    evaluate('return getComputedStyle(document.querySelector(".sim-person")).backgroundColor'),
    "rgba(0, 0, 0, 0)",
    "Production indicates speech on the avatar, not a green row",
  );
  assert.equal(
    evaluate('return document.querySelectorAll(".live-window .participant-country").length'),
    0,
    "The simulation has no country flags",
  );
  assert.deepEqual(speakers(), ["Maya", "Theo", "June"], "Multiple people speak simultaneously");
  assert.equal(evaluate('return document.querySelector(".voice-stack").tagName'), "SPAN");
  evaluate('window.firstDemoMessage = document.querySelector(".sim-message");');

  // Joining a channel must not automatically join voice; leaving voice keeps
  // membership. Drive the real timer callback through the local clock fixture.
  assert.deepEqual(members(), ["Maya", "Theo", "June"]);
  nextMoments(1); // 4
  assert.deepEqual(members(), ["Maya", "Theo", "June", "Leo"]);
  assert.deepEqual(voices(), ["Maya", "Theo", "June"]);
  assert.deepEqual(speakers(), ["Maya", "Theo", "June"]);
  nextMoments(1); // 5: Maya stops while Theo and June continue
  assert.deepEqual(speakers(), ["Theo", "June"]);
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-message:last-of-type .sim-reaction").length'),
    0,
    "Leo posted at 4.75; no bundled reaction",
  );
  nextMoments(1); // 6: Leo's message is 1.25 seconds old
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-message:last-of-type .sim-reaction").length'),
    0,
    "Readers have not reacted yet",
  );
  nextMoments(1); // 7: Leo's message is 2.25 seconds old
  assert.deepEqual(
    evaluate(
      'return [...document.querySelectorAll(".sim-message:last-of-type .sim-reaction > span")].map(el => Number(el.textContent))',
    ),
    [1],
    "First reaction arrives after the message",
  );
  nextMoments(1); // 8
  assert.ok(members().includes("Noor"));
  assert.ok(!voices().includes("Noor"));
  assert.equal(evaluate('return document.querySelector(".sim-typing").textContent.trim()'), "Noor and Theo are typing");
  assert.deepEqual(speakers(), ["Maya", "Theo"]);
  nextMoments(2); // 10
  assert.ok(voices().includes("Leo"));
  checkVoiceSummary(["Maya", "Theo", "June", "Leo"]);
  nextMoments(1); // 11
  assert.deepEqual(speakers(), ["June", "Leo"]);
  nextMoments(1); // 12: two messages arrive in the same second
  assert.deepEqual(
    evaluate('return [...document.querySelectorAll(".sim-message p")].slice(-2).map(el => el.textContent)'),
    ["already here. bringing the playlist 🎶", "save me a spot"],
  );
  assert.deepEqual(speakers(), ["Maya", "June", "Leo"]);
  nextMoments(3); // 15
  assert.equal(members().length, 6);
  assert.ok(!voices().includes("Sam"));
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-message").length'),
    9,
    "Keep every posted message, not just the latest three",
  );
  assert.ok(
    evaluate(
      'return window.firstDemoMessage === document.querySelector(".sim-message") && window.firstDemoMessage.textContent.includes("okay, this made my entire morning")',
    ),
  );
  assert.ok(
    evaluate(
      'const el = document.querySelector(".sim-messages"); return el.scrollHeight > el.clientHeight && el.scrollHeight - el.scrollTop - el.clientHeight <= 1;',
    ),
    "Full-height history follows new posts",
  );
  browser("scroll", "up", "10000", "--selector", ".sim-messages");
  wait('document.querySelector(".sim-messages").scrollTop === 0');
  nextMoments(7); // 22
  assert.ok(members().includes("Maya"));
  assert.ok(!voices().includes("Maya"));
  assert.equal(
    evaluate('return document.querySelector(".sim-sidebar .voice-dock")'),
    null,
    "The voice dock reflects Maya leaving voice",
  );
  checkVoiceSummary(["Theo", "June", "Leo", "Noor"]);
  assert.equal(
    evaluate('return document.querySelector(".sim-messages").scrollTop'),
    0,
    "Reading older messages is not interrupted by new posts",
  );
  evaluate(
    'const el = document.querySelector(".sim-messages"); el.scrollTop = el.scrollHeight; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));',
  );
  nextMoments(16); // 38
  assert.equal(voices().length, 6);
  checkVoiceSummary(["Maya", "Theo", "June", "Leo", "Noor", "Sam"]);
  nextMoments(1); // 39
  assert.ok(!members().includes("June"));
  assert.ok(!voices().includes("June"));
  checkVoiceSummary(["Maya", "Theo", "Leo", "Noor", "Sam"]);
  nextMoments(6); // 45
  assert.equal(members().length, 6);
  assert.equal(voices().length, 6);
  checkVoiceSummary(["Maya", "Theo", "June", "Leo", "Noor", "Sam"]);
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-person .participant-country").length'),
    0,
    "Joining/rejoining demo participants have no flags",
  );
  assert.ok(
    evaluate('return document.querySelector(".sim-messages").textContent.includes("back with cookies. let’s gooo")'),
  );
  assert.equal(evaluate('return document.querySelectorAll(".sim-message").length'), 24);
  nextMoments(4); // 0: new loop, but the previous conversation stays
  assert.deepEqual(members(), ["Maya", "Theo", "June"]);
  assert.deepEqual(voices(), ["Maya", "Theo", "June"]);
  checkVoiceSummary(["Maya", "Theo", "June"]);
  assert.equal(evaluate('return document.querySelectorAll(".sim-message").length'), 25);
  assert.ok(
    evaluate('return window.firstDemoMessage === document.querySelector(".sim-message")'),
    "Keep the same message nodes through the loop boundary",
  );
  assert.equal(
    evaluate('return document.querySelectorAll(".sim-message:last-of-type .sim-reaction").length'),
    0,
    "New loop message starts without reactions",
  );
  assert.equal(
    evaluate(
      'return Number(document.querySelectorAll(".sim-message")[23].querySelector(".sim-reaction > span").textContent)',
    ),
    1,
    "Older messages do not receive all remaining reactions instantly at the loop boundary",
  );
  nextMoments(3); // June posts again; her original message remains too
  assert.equal(evaluate('return document.querySelectorAll(".sim-message:last-of-type .sim-reaction").length'), 0);
  const firstCycleReactions = () =>
    evaluate(
      'return [...document.querySelectorAll(".sim-message")].slice(0, 24).map(message => Number(message.querySelector(".sim-reaction > span")?.textContent ?? 0))',
    );
  const expectedReactions = [3, 0, 0, 1, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 2, 0, 0, 3, 0, 2];
  assert.deepEqual(
    firstCycleReactions(),
    expectedReactions,
    "Only seven standout posts receive reactions, with varied counts; ordinary messages stay unreacted after every delay has elapsed",
  );
  nextMoments(49); // A second complete loop still cannot erase the original
  assert.equal(evaluate('return document.querySelectorAll(".sim-message").length'), 51);
  assert.ok(evaluate('return window.firstDemoMessage === document.querySelector(".sim-message")'));
  assert.deepEqual(
    firstCycleReactions(),
    expectedReactions,
    "Later loops cannot add reactions to ordinary messages or inflate the original counts",
  );
  assert.ok(
    evaluate(
      'const el = document.querySelector(".sim-messages"); return el.scrollHeight - el.scrollTop - el.clientHeight <= 1;',
    ),
  );
  console.log("PASS: 17 of 24 messages never receive reactions; seven standout posts get 1–3 delayed reactions");
  console.log(
    "PASS: full message history persists across two loops; auto-follow and reading older messages work; reactions remain delayed and read-only",
  );
  console.log(
    "PASS: general/feedback channels, no join notices/hints/actions, nested voice roster, simultaneous speakers, burst messages and multi-person typing",
  );

  // Resize after history checks: changing the scroll viewport can stop following.
  nextMoments(35); // 38: all six people are in voice
  for (const width of [320, 390]) {
    browser("set", "viewport", String(width), "844", "2");
    evaluate("await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));");
    checkVoiceSummary(["Maya", "Theo", "June", "Leo", "Noor", "Sam"]);
  }
  browser("set", "viewport", "1280", "800", "2");
  evaluate("await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));");

  // Mock only clipboard rejection: browsers may deny it, and we must not claim success.
  evaluate('navigator.clipboard.writeText = async () => { throw new Error("Test denial"); };');
  browser("click", "button.contact-action");
  wait('document.querySelector(".contact-feedback").textContent.includes("Couldn’t copy")');
  assert.ok(
    evaluate('return document.querySelector(".contact-feedback").textContent.includes("contact@josevalerio.com")'),
  );
  assert.deepEqual(
    evaluate(
      'return performance.getEntriesByType("resource").filter(r => /\\/api\\/(chat|media|channels)/.test(r.name)).map(r => r.name)',
    ),
    [],
  );
  browser("click", ".live-invite");
  wait('document.querySelector("#email")');
  assert.equal(evaluate("return location.pathname"), "/login");
  const login = evaluate('return document.querySelector("main").textContent');
  assert.equal(evaluate('return document.querySelector("h1").textContent'), "Welcome to Caper");
  assert.ok(login.includes("We’ll send a code to your email."));
  assert.ok(!/WELCOME TO CAPER|We only send a code when you ask|No password needed/.test(login));
  browser("network", "route", "**/api/auth/email/request", "--body", JSON.stringify({ challengeId: "layout-fixture" }));
  const loginGaps = () =>
    evaluate(`
    const heading = document.querySelector('h1');
    const input = document.querySelector('input');
    return [
      heading.nextElementSibling.getBoundingClientRect().top - heading.getBoundingClientRect().bottom,
      document.querySelector('button[type=submit]').getBoundingClientRect().top - input.getBoundingClientRect().bottom,
    ];
  `);
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "844", "2");
    evaluate(
      "await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));",
    );
    assert.deepEqual(loginGaps(), [4, 12], `Email entry gaps at ${width}px`);
    const action = evaluate(`
      const input = document.querySelector('#email').getBoundingClientRect();
      const button = document.querySelector('button[type=submit]').getBoundingClientRect();
      return { right: button.right - input.right, width: button.width, inputWidth: input.width };
    `);
    assert.equal(action.right, 0, `Email action aligns with input's right edge at ${width}px`);
    assert.ok(action.width < action.inputWidth, `Email action is compact at ${width}px`);
    browser("fill", "#email", "layout@example.test");
    browser("click", "button[type=submit]");
    wait('document.querySelector("#code")');
    assert.deepEqual(loginGaps(), [4, 12], `Verification gaps at ${width}px`);
    browser("click", "button[type=button]");
    wait('document.querySelector("#email")');
  }
  console.log(
    "PASS: email/verification title gaps are 4px and input/action gaps are 12px at desktop and narrow widths",
  );
  browser("set", "viewport", "390", "844", "2");
  // Navigation clicks must not race the card's arrival/idle animation. Motion
  // and dragging were exercised above; use the stable view for account fixtures.
  browser("set", "media", "dark", "reduced-motion");
  browser("open", origin.href);
  wait(
    'document.querySelector(".live-stage[data-ready]") && typeof window.advanceDemoClock === "function" && !window.demoClockActive',
  );
  evaluate(
    "await document.fonts.ready; await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));",
  );
  checkVoiceSummary(["Maya", "Theo", "June"]);
  console.log(
    "PASS: all 3–6 voice avatars stay visible through joins, leaves and loop resets; desktop, 320px and 390px totals are not clipped",
  );
  browser("scrollintoview", ".sim-message:last-of-type p");
  checkReadOnlyHover([
    ".sim-message:last-of-type",
    ".sim-reaction",
    ".sim-person",
    ".account-profile",
    ".voice-icon-button",
    ".call-settings-trigger",
    ".member-list-toggle",
  ]);
  drag(".sim-message:last-of-type p");
  assert.equal(evaluate('return getComputedStyle(document.querySelector(".sim-messages")).cursor'), "grab");
  const gap = evaluate(
    'return document.querySelector(".live-scene").getBoundingClientRect().top - document.querySelector(".hero-lede").getBoundingClientRect().bottom',
  );
  assert.ok(gap >= 40, `Mobile demo needs clearance above its projected edge; got ${gap}px`);
  assert.notEqual(
    evaluate(
      'return getComputedStyle(document.querySelector(".sim-sidebar .channel-navigation > ul > li:last-child")).display',
    ),
    "none",
    "Feedback stays visible on mobile",
  );
  // Local account fixtures: a returning member must never visit the login route.
  browser("network", "unroute", "**/api/account/me");
  browser(
    "network",
    "route",
    "**/api/account/me",
    "--body",
    JSON.stringify({ id: "demo-test", username: "demo-test", displayName: "Demo test" }),
  );
  browser(
    "network",
    "route",
    "**/api/spaces",
    "--body",
    JSON.stringify({ spaces: [], limits: { ownedSpaces: 10, totalSpaces: 10, channelsPerSpace: 10 } }),
  );
  browser("network", "route", "**/login", "--body", "<h1>Unexpected login navigation</h1>");
  browser("click", ".live-invite");
  wait('document.querySelector(".spaces-empty")');
  assert.equal(evaluate("return location.pathname"), "/spaces");
  assert.equal(evaluate('return !!document.querySelector("#email")'), false);
  browser("network", "unroute", "**/api/account/me");
  browser(
    "network",
    "route",
    "**/api/account/me",
    "--body",
    JSON.stringify({ id: "demo-test", username: null, displayName: null }),
  );
  browser("open", origin.href);
  wait('document.querySelector(".live-stage[data-ready]")');
  browser("click", ".live-invite");
  wait('location.pathname === "/profile"');
  console.log(
    "PASS: centered join-only CTA, reduced motion, clipboard failure, no chat/media requests and email login",
  );
  console.log("PASS: non-blocking font loading and no in-room simulation badge");
  console.log("PASS: simplified login copy and mobile demo clearance");
  console.log("PASS: signed-in join bypasses login; incomplete accounts reach profile setup (mocked accounts)");
} catch (error) {
  console.error("Homepage regression failed at", evaluate("return location.pathname"), browser("snapshot"));
  throw error;
} finally {
  browser("close");
  rmSync(fixture, { recursive: true, force: true });
}
