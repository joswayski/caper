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
  click("Invite people");
  wait('!!document.querySelector(".invite-link-share input")');
};
const close = () => click("Close Invite people to Fixture Studio");
const capture = (name) => {
  if (artifacts) browser("screenshot", `${artifacts}/${name}.png`);
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
  assert.match(text(), /100 people.*16 days/);
  assert.doesNotMatch(text(), /Space name|Save name|Delete space|Remove/);
  assert.equal(evaluate('return document.querySelector(".invite-link-settings").open;'), false);
  click("Copy link");
  wait('[...document.querySelectorAll("dialog button")].some(b => b.textContent === "Copied")');
  close();
  open();
  assert.equal(url(), initial, "Reopening must keep the link");
  capture("web-invite-default");
  browser("click", ".invite-link-settings summary");
  assert.equal(evaluate('return document.querySelector(".invite-link-expiry input").value;'), "16");
  assert.equal(evaluate('return document.querySelector(".invite-link-options label input").value;'), "100");
  capture("web-invite-settings");
  browser("fill", 'input[aria-label="Expire after amount"]', "4");
  browser("select", 'select[aria-label="Expire after unit"]', "hours");
  browser("fill", 'input[placeholder="No limit"]', "17");
  browser("select", ".invite-link-options label select", "chan00000002");
  click("Create link");
  wait('document.querySelector(".invite-links ul")?.textContent.includes("17 uses")');
  assert.match(text(), /#design.*0 of 17 uses.*Expires in 4 hours/);
  assert.equal(url(), initial, "Custom creation must not replace the default");
  click("Deactivate link");
  wait('!document.querySelector(".invite-link-share input")');
  assert.match(text(), /No active default link/);
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
  browser("click", ".invite-link-settings summary");
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
  console.log(
    "PASS: invite-only UI, copy, reuse, custom link, deactivation/replacement, narrow layout/scroll, username invite and retry.",
  );
} finally {
  browser("close");
}
