// Real web interactions against the labelled disposable fixture, not live accounts.
// Start native-parity-fixture.mjs and Vite, then pass Vite's loopback URL.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const origin = new URL(process.argv[2] ?? "http://127.0.0.1:5174");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname), "Loopback only");
// Synchronous browser commands pause Node's event loop: do not keep HTTP
// sockets in a pool whose close events cannot be processed during those calls.
assert.equal(
  (await fetch("http://127.0.0.1:3001/health", { headers: { connection: "close" } }).then((r) => r.json())).fixture,
  true,
);
async function control(body) {
  const response = await fetch("http://127.0.0.1:3001/__fixture/control", {
    method: "POST",
    headers: { "content-type": "application/json", connection: "close" },
    body: JSON.stringify(body),
  });
  assert.ok(response.ok);
  return response.json();
}
await control({ reset: true });
const session = `invites-${process.pid}`;
const artifacts = process.env.INVITE_TEST_ARTIFACTS && resolve(process.env.INVITE_TEST_ARTIFACTS);
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
const text = () => evaluate('return document.querySelector("dialog[open]").textContent;');
const url = () => evaluate('return document.querySelector(".invite-link-share input").value;');
const click = (name) => browser("find", "role", "button", "click", "--name", name, "--exact");
const open = () => {
  browser("click", ".space-menu summary");
  click("Invite people…");
  wait('!!document.querySelector(".invite-link-more:not(:disabled)")');
};
const close = () => click("Close Invite people to Fixture Studio");
const options = () => {
  click("More invite options");
  wait('!!document.querySelector(".invite-link-menu")');
};
const settings = () => {
  options();
  click("Custom links");
  wait('document.querySelector(".invite-link-settings")?.open');
};
const confirm = (action) => {
  wait('document.querySelectorAll("dialog[open]").length === 2');
  assert.equal(evaluate("return document.activeElement?.textContent;"), "Cancel");
  click(action);
  wait('document.querySelectorAll("dialog[open]").length === 1');
};
const capture = (name) => {
  if (!artifacts) return;
  evaluate(
    'return Promise.all(document.querySelector("dialog[open]").getAnimations({subtree:true}).filter(a => a.effect?.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {})));',
  );
  browser("screenshot", "dialog[open]:not(:has(dialog[open]))", `${artifacts}/${name}.png`);
};
try {
  browser("open", origin.href);
  browser("cookies", "set", "caper_fixture", "owner");
  browser("open", new URL("/spaces", origin).href);
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  browser("set", "viewport", "1280", "900", "2");
  evaluate(
    "return document.fonts.ready.then(() => Promise.all(document.getAnimations().filter(a => a.effect?.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))));",
  );
  open();
  const initial = url();
  assert.match(initial, /\/invite\/[a-zA-Z0-9]{10}$/);
  assert.match(text(), /Anyone with this link can join this space.*100 joins.*30 days/);
  assert.match(text(), /#general · 100 of 100 joins remaining · Expires in 30 days/);
  assert.equal(
    evaluate('return [...document.querySelectorAll("dialog select")].some(select => select.checkVisibility());'),
    false,
    "The normal invite flow has no destination or settings picker",
  );
  assert.doesNotMatch(text(), /\bdefault\b/i);
  assert.doesNotMatch(text(), /Space name|Save name|Delete space|Remove/);
  assert.equal(evaluate('return document.querySelector(".invite-link-settings").open;'), false);
  click("Copy");
  wait('[...document.querySelectorAll("dialog button")].some(b => b.textContent === "Copied")');
  close();
  open();
  assert.equal(url(), initial, "Reopening must keep the link");
  capture("web-invite-default");
  options();
  assert.match(text(), /The old link stops working/);
  capture("web-invite-options");
  browser("press", "Escape");
  wait('!document.querySelector(".invite-link-menu")');
  assert.equal(evaluate('return document.activeElement?.getAttribute("aria-label");'), "More invite options");
  assert.equal(url(), initial, "Escape closes options, not the invitation dialog");
  browser("press", "Enter");
  wait('!!document.querySelector(".invite-link-menu")');
  assert.equal(evaluate('return document.activeElement?.getAttribute("aria-label");'), "Regenerate link");
  browser("press", "Tab");
  assert.match(evaluate("return document.activeElement?.textContent;"), /Custom links/);
  browser("press", "Escape");
  const initialCode = initial.split("/").at(-1);
  options();
  click("Regenerate link");
  wait('document.querySelectorAll("dialog[open]").length === 2');
  assert.equal(evaluate("return document.activeElement?.textContent;"), "Cancel");
  assert.doesNotMatch(text(), /Existing members (stay|remain)/);
  capture("web-invite-regenerate-confirmation");
  browser("press", "Enter");
  wait('document.querySelectorAll("dialog[open]").length === 1');
  assert.equal(url(), initial, "Enter initially cancels instead of invalidating a shared URL");
  const stillLive = await fetch("http://127.0.0.1:3001/api/spaces/space0000001/invite-links", {
    headers: { cookie: "caper_fixture=owner", connection: "close" },
  }).then((r) => r.json());
  assert.deepEqual(
    stillLive.links.map((link) => link.code),
    [initialCode],
    "Cancellation must not mutate live links",
  );
  await control({
    failure: {
      path: `/api/spaces/space0000001/invite-links/${initialCode}/regenerate`,
      method: "POST",
      status: 503,
      persistent: true,
    },
  });
  options();
  click("Regenerate link");
  confirm("Regenerate link");
  wait('!!document.querySelector("dialog[open] [role=alert]")');
  assert.equal(url(), initial, "A failed regeneration preserves the displayed link");
  capture("web-invite-regenerate-error");
  await control({ clearFailures: true });
  options();
  click("Regenerate link");
  confirm("Regenerate link");
  wait(`document.querySelector('.invite-link-share input')?.value !== ${JSON.stringify(initial)}`);
  const regenerated = url();
  assert.match(text(), /100 of 100 joins remaining · Expires in 30 days/);
  const live = await fetch("http://127.0.0.1:3001/api/spaces/space0000001/invite-links", {
    headers: { cookie: "caper_fixture=owner", connection: "close" },
  }).then((r) => r.json());
  assert.equal(
    live.links.some((link) => link.code === initialCode),
    false,
  );
  assert.equal(live.links.length, 1);
  assert.equal(live.links[0].channel.id, "chan00000001", "Regeneration retains the default destination");
  close();
  open();
  assert.equal(url(), regenerated, "Reopening selects the regenerated code");
  capture("web-invite-regenerated");
  settings();
  assert.equal(evaluate('return document.querySelector(".invite-link-expiry input").value;'), "30");
  assert.equal(evaluate('return document.querySelector(".invite-link-options label input").value;'), "100");
  assert.equal(
    evaluate('return document.querySelector(".invite-link-options label select option:checked").textContent;'),
    "#general",
  );
  assert.doesNotMatch(text(), /\bdefault\b/i);
  capture("web-invite-settings");
  browser("fill", 'input[aria-label="Expire after amount"]', "4");
  browser("select", 'select[aria-label="Expire after unit"]', "hours");
  browser("fill", 'input[placeholder="No limit"]', "17");
  browser("select", ".invite-link-options label select", "chan00000002");
  click("Create link");
  wait('document.querySelector(".invite-links ul")?.textContent.includes("17 joins remaining")');
  assert.match(text(), /#design.*17 of 17 joins remaining.*Expires in 4 hours/);
  assert.equal(url(), regenerated, "Custom creation must not replace the default");
  // List actions must identify and deactivate only that separate link.
  browser("click", ".invite-links li:first-child .invite-link-actions button:last-child");
  confirm("Deactivate link");
  wait('!document.querySelector(".invite-links ul")?.textContent.includes("17 joins remaining")');
  assert.equal(url(), regenerated, "Deactivating a custom link preserves the primary link");
  options();
  click("Deactivate link");
  wait('document.querySelectorAll("dialog[open]").length === 2');
  capture("web-invite-deactivate-confirmation");
  browser("press", "Escape");
  wait('document.querySelectorAll("dialog[open]").length === 1');
  assert.equal(url(), regenerated, "Escape dismisses only confirmation, preserving the shared link");
  options();
  click("Deactivate link");
  confirm("Deactivate link");
  wait('!document.querySelector(".invite-link-share input")');
  assert.match(text(), /No active invite link/);
  capture("web-invite-deactivated");
  click("Create another link");
  wait('!!document.querySelector(".invite-link-share input")');
  const replacement = url();
  assert.notEqual(replacement, initial);
  close();
  open();
  assert.equal(url(), replacement);
  browser("set", "viewport", "390", "844", "2");
  evaluate("return new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));");
  capture("web-invite-narrow");
  options();
  const menu = evaluate(
    'const r=document.querySelector(".invite-link-menu").getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};',
  );
  assert.ok(menu.left >= 0 && menu.right <= 390 && menu.top >= 0 && menu.bottom <= 844, JSON.stringify(menu));
  capture("web-invite-options-narrow");
  click("Deactivate link");
  wait('document.querySelectorAll("dialog[open]").length === 2');
  capture("web-invite-confirmation-narrow");
  click("Cancel");
  wait('document.querySelectorAll("dialog[open]").length === 1');
  settings();
  capture("web-invite-settings-narrow");
  browser("scrollintoview", '.invite-link-options button[type="submit"]');
  const bounds = evaluate(
    'const r=document.querySelector(".invite-link-options button[type=submit]").getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom};',
  );
  assert.ok(bounds.left >= 0 && bounds.right <= 390 && bounds.top >= 0 && bounds.bottom <= 844, JSON.stringify(bounds));
  capture("web-invite-settings-narrow-scrolled");
  browser("scroll", "up", "2000", "--selector", "dialog[open]");
  browser("click", ".invite-link-settings summary");
  browser("fill", "#member-username", "sam");
  click("Invite");
  wait('document.querySelector("dialog[open]")?.textContent.includes("Invitation sent")');
  assert.match(text(), /Pending invitations/);
  assert.doesNotMatch(text(), /Delete space|Save name|Remove/);
  close();
  browser("set", "viewport", "1280", "900", "2");
  await control({
    failure: { path: "/api/spaces/space0000001/invite-links/default", method: "POST", status: 503, persistent: true },
  });
  browser("click", ".space-menu summary");
  click("Invite people…");
  wait('!!document.querySelector("dialog[open] [role=alert]")');
  capture("web-invite-error");
  await control({ clearFailures: true });
  click("Try again");
  wait('!!document.querySelector(".invite-link-share input")');
  assert.equal(url(), replacement);

  // Rename and privacy changes must update the displayed destination, not a
  // hard-coded general or a channel sorted by its random external ID.
  const changeChannel = async (id, name, privateChannel) => {
    const response = await fetch(`http://127.0.0.1:3001/api/spaces/space0000001/channels/${id}`, {
      method: "PATCH",
      headers: { cookie: "caper_fixture=owner", "content-type": "application/json", connection: "close" },
      body: JSON.stringify({ name, private: privateChannel }),
    });
    assert.ok(response.ok);
  };
  const reopenSpace = () => {
    browser("open", new URL("/spaces", origin).href);
    wait('!!document.querySelector("#chat-message:not(:disabled)")');
    evaluate(
      "return document.fonts.ready.then(() => Promise.all(document.getAnimations().filter(a => a.effect?.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))));",
    );
    browser("set", "viewport", "1280", "900", "2");
    open();
  };
  close();
  await changeChannel("chan00000001", "welcome", false);
  await changeChannel("chan00000002", "general", false);
  reopenSpace();
  // The server picks the oldest public channel, not the newer one named general.
  assert.match(text(), /#welcome · 100 of 100 joins remaining/);
  close();
  await changeChannel("chan00000001", "welcome", true);
  reopenSpace();
  assert.match(text(), /#general · 100 of 100 joins remaining/, "Private channels are excluded");
  capture("web-invite-default-fallback");
  settings();
  assert.equal(
    evaluate('return document.querySelector(".invite-link-options label select option:checked").textContent;'),
    "#general",
  );
  close();
  await changeChannel("chan00000002", "general", true);
  reopenSpace();
  assert.match(text(), /People will join the space without opening a channel/);
  capture("web-invite-no-public-channel");
  settings();
  assert.equal(
    evaluate('return document.querySelector(".invite-link-options label select option:checked").textContent;'),
    "No public channels",
  );
  close();
  await control({ reset: true });
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    browser("cookies", "set", "caper_fixture", "owner");
    browser("open", new URL("/spaces?space=space0000001&channel=chan00000001", origin).href);
    wait('!!document.querySelector("#chat-message:not(:disabled)")');
    if (width === 390) {
      click("Back to Browse");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room").hasAttribute("data-browse-slide")',
      );
    }
    browser("click", ".direct-action");
    assert.match(text(), /New direct message/);
    click("Close New direct message");
    open();
    const spaceLink = url();
    assert.match(text(), /#general · 100 of 100 joins remaining/);
    capture(`menu-space-invite-${width}`);
    close();
    browser("click", 'summary[aria-label="Manage design"]');
    click("Invite people…");
    wait('!!document.querySelector(".invite-link-more:not(:disabled)")');
    const channelLink = url();
    assert.notEqual(channelLink, spaceLink);
    assert.match(text(), /#design · 100 of 100 joins remaining/);
    assert.equal(
      evaluate('return document.querySelector(".channel-select[aria-current=page]").textContent;'),
      "general",
    );
    capture(`menu-channel-invite-${width}`);
    close();
    browser("click", 'summary[aria-label="Manage design"]');
    click("Invite people…");
    wait('!!document.querySelector(".invite-link-more:not(:disabled)")');
    assert.equal(url(), channelLink, "Channel invitations reuse their prepared link");
    close();
    open();
    assert.equal(url(), spaceLink, "Space invitations do not inherit the last clicked channel");
    close();
    browser("click", 'summary[aria-label="Manage planning"]');
    assert.equal(
      evaluate('return document.querySelector(".channel-menu[open]").textContent.includes("Invite people");'),
      false,
    );
    browser("press", "Escape");
    browser("cookies", "set", "caper_fixture", "other");
    browser("open", new URL("/spaces?space=space0000001&channel=chan00000001", origin).href);
    wait('!!document.querySelector("#chat-message:not(:disabled)")');
    if (width === 390) {
      click("Back to Browse");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room").hasAttribute("data-browse-slide")',
      );
    }
    browser("click", ".space-menu summary");
    assert.equal(
      evaluate('return document.querySelector(".space-menu").textContent.includes("Invite people");'),
      false,
    );
    browser("press", "Escape");
    browser("click", 'summary[aria-label="Manage design"]');
    assert.equal(
      evaluate('return document.querySelector(".channel-menu[open]").textContent.includes("Invite people");'),
      false,
    );
  }
  console.log(
    "PASS: owner-only space/public-channel invite menus, clicked-channel link reuse without navigation, DM footer; copy-first invites without a main destination picker, safe regeneration/deactivation, custom links, failure recovery, 30-day/100-join defaults, narrow layout and username invitations.",
  );
} finally {
  browser("close");
}
