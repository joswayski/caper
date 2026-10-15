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
const destination = () => evaluate('return document.querySelector(".invite-link-destination select").value;');
const chooseChannel = (id) => {
  browser("select", ".invite-link-destination select", id);
  wait('!!document.querySelector(".invite-link-more:not(:disabled)")');
};
const click = (name) => browser("find", "role", "button", "click", "--name", name, "--exact");
const open = () => {
  click("Invite people");
  wait('!!document.querySelector(".invite-link-more:not(:disabled)")');
};
const close = () => click("Close Invite people to Fixture Studio");
const options = () => {
  click("More invite options");
  wait('!!document.querySelector(".invite-link-menu")');
};
const settings = () => {
  options();
  click("Link settings");
  wait('document.querySelector(".invite-link-settings")?.open');
};
const capture = (name) => {
  if (!artifacts) return;
  evaluate(
    'return Promise.all(document.querySelector("dialog[open]").getAnimations({subtree:true}).filter(a => a.effect?.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {})));',
  );
  browser("screenshot", "dialog[open]", `${artifacts}/${name}.png`);
};
try {
  browser("open", origin.href);
  browser("cookies", "set", "caper_fixture", "owner");
  browser("open", new URL("/spaces", origin).href);
  wait('!!document.querySelector("#chat-message:not(:disabled)")');
  browser("set", "viewport", "1280", "900", "2");
  open();
  const initial = url();
  assert.match(initial, /\/invite\/[a-zA-Z0-9]{10}$/);
  assert.match(text(), /100 people.*30 days/);
  assert.match(text(), /100 of 100 joins remaining · Expires in 30 days/);
  assert.equal(destination(), "chan00000001");
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
  assert.match(evaluate("return document.activeElement?.textContent;"), /Link settings/);
  browser("press", "Escape");
  const initialCode = initial.split("/").at(-1);
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
  wait('!!document.querySelector("dialog[open] [role=alert]")');
  assert.equal(url(), initial, "A failed regeneration preserves the displayed link");
  capture("web-invite-regenerate-error");
  await control({ clearFailures: true });
  options();
  click("Regenerate link");
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
  close();
  open();
  assert.equal(url(), regenerated, "Reopening selects the regenerated code");
  capture("web-invite-regenerated");
  chooseChannel("chan00000002");
  const designLink = url();
  assert.notEqual(designLink, regenerated);
  assert.equal(destination(), "chan00000002");
  assert.match(text(), /#design · 100 of 100 joins remaining/);
  capture("web-invite-channel-selected");
  await control({
    failure: { path: "/api/spaces/space0000001/invite-links/default", method: "POST", status: 503, persistent: true },
  });
  chooseChannel("chan00000001");
  assert.equal(url(), designLink, "Failed selection preserves the previous link");
  assert.equal(destination(), "chan00000002", "Failed selection preserves the previous channel");
  assert.match(text(), /requested failure/);
  capture("web-invite-channel-error");
  await control({ clearFailures: true });
  options();
  click("Regenerate link");
  wait(`document.querySelector('.invite-link-share input')?.value !== ${JSON.stringify(designLink)}`);
  assert.equal(destination(), "chan00000002", "Regeneration retains the chosen channel");
  chooseChannel("chan00000001");
  assert.equal(url(), regenerated, "Switching back reuses the unchanged old link");
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
  click("Deactivate link");
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
  click("Link settings");
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
  click("Invite people");
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
  assert.equal(destination(), "chan00000001", "Oldest public channel wins even when a newer one is named general");
  assert.match(text(), /#welcome · 100 of 100 joins remaining/);
  close();
  await changeChannel("chan00000001", "welcome", true);
  reopenSpace();
  assert.equal(destination(), "chan00000002", "Private channels are excluded");
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
  assert.equal(evaluate('return document.querySelector(".invite-link-destination select").disabled;'), true);
  capture("web-invite-no-public-channel");
  settings();
  assert.equal(
    evaluate('return document.querySelector(".invite-link-options label select option:checked").textContent;'),
    "No public channels",
  );
  console.log(
    "PASS: channel picker, oldest public selection, preserved shared links, selection failure recovery, regeneration retaining channel, no-public-channel state, 30-day/100-join defaults, copy, keyboard options, custom links, deactivation, narrow layout/scroll and username invitations.",
  );
} finally {
  browser("close");
}
