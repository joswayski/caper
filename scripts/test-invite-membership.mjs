// Real invite UI with explicitly mocked account/invite responses; never joins a real space.
// Start the web dev server, then: node scripts/test-invite-membership.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const origin = process.env.INVITE_TEST_WEB_URL ?? "http://localhost:5174";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(origin).hostname), "Use a disposable loopback preview");
const directory = mkdtempSync(join(tmpdir(), "caper-invite-"));
const init = join(directory, "fixture.js");
const artifacts = process.env.INVITE_TEST_ARTIFACTS && resolve(process.env.INVITE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });

function fixture() {
  if (location.protocol === "about:") return;
  const mode = new URL(location.href).searchParams.get("fixture");
  const account = { id: "account00001", username: "fixture", displayName: "TEST FIXTURE member", avatarId: 3 };
  const space = { id: "space0000001", name: "TEST FIXTURE · Invite", ownerId: "owner0000001" };
  const state = (window.inviteFixture = { posts: 0, account: mode === "guest" ? null : account });
  const original = window.fetch.bind(window);
  window.fetch = async (input, options) => {
    const path = new URL(input instanceof Request ? input.url : String(input), location.href).pathname;
    if (!path.startsWith("/api/")) return original(input, options);
    if (path === "/api/account/me") {
      if (mode === "outage") return Response.json({ error: "unavailable" }, { status: 503 });
      return state.account ? Response.json(state.account) : Response.json({ error: "unauthorized" }, { status: 401 });
    }
    if (path === "/api/auth/email/request") return Response.json({ challengeId: "TEST-challenge" });
    if (path === "/api/auth/email/verify") {
      state.account = { ...account, username: null, displayName: null };
      return Response.json({ account: state.account });
    }
    if (path === "/api/account/profile") {
      state.account = { ...account, ...JSON.parse(options.body) };
      return Response.json(state.account);
    }
    if (path === "/api/invites/TESTfixt01")
      return Response.json({
        code: "TESTfixt01",
        space: { name: space.name },
        channel: { name: "feedback" },
        inviter: { username: "fixture_host", displayName: "TEST FIXTURE host", avatarId: 7 },
        memberCount: 2,
        expiresAt: null,
        ...(mode === "legacy" ? {} : { alreadyMember: mode === "member" }),
      });
    if (path === "/api/invites/TESTfixt01/join" && options?.method === "POST") {
      state.posts++;
      return Response.json({ space, channelId: "channel00001", joined: mode !== "member" });
    }
    if (path === "/api/spaces")
      return Response.json({
        spaces: [space],
        invitations: [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path === `/api/spaces/${space.id}`)
      return Response.json({ space, channels: [], members: [{ ...account, owner: false }] });
    if (path === "/api/dms") return Response.json({ conversations: [] });
    return Response.json({ error: "TEST FIXTURE: disabled endpoint" }, { status: 503 });
  };
}
writeFileSync(init, `(${fixture.toString()})()`);
const browser = (...command) => {
  const response = JSON.parse(
    execFileSync("agent-browser", ["--session", "invite-membership", "--init-script", init, ...command, "--json"], {
      encoding: "utf8",
      timeout: 60_000,
    }),
  );
  assert.ok(response.success, response.error);
  return response.data;
};
const evaluate = (code) => browser("eval", code).result;

try {
  browser("open", "about:blank");
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "844", "2");
    // A reloaded profile with an expired session must keep its invite through reauthentication.
    browser("open", `${origin}/profile?fixture=guest&invite=TESTfixt01`);
    browser("wait", "--url", "**/login*");
    assert.equal(evaluate("location.search"), "?invite=TESTfixt01");
    browser("wait", "#email");
    browser("find", "label", "Email address", "fill", "invite-fixture@example.test");
    browser("find", "role", "button", "click", "--name", "Email me a code");
    browser("wait", "#code");
    browser("find", "label", "Sign-in code", "fill", "ABC234");
    browser("press", "Enter");
    browser("wait", "--url", "**/profile?invite=TESTfixt01");
    browser("wait", "#username");
    browser("find", "label", "Username", "fill", "invite_fixture");
    browser("find", "label", "Display name", "fill", "TEST FIXTURE member");
    browser("find", "role", "button", "click", "--name", "Finish account");
    browser("wait", "--url", "**/invite/TESTfixt01");
    browser("wait", "--fn", 'document.querySelector("main button")?.textContent.includes("Join space")');
    assert.equal(evaluate("inviteFixture.posts"), 0, "Reauthentication must not automatically accept the invite");
    browser("click", "main button");
    browser("wait", "--url", "**/spaces?space=space0000001&channel=channel00001");
    assert.equal(evaluate("inviteFixture.posts"), 1);

    for (const invite of ["", "&invite=bad", "&invite=%2F%2Fevil.example"]) {
      browser("open", `${origin}/profile?fixture=guest${invite}`);
      browser("wait", "--url", "**/login");
      assert.equal(evaluate("location.search"), "", "Only a valid invite may survive the redirect");
    }
    browser("open", `${origin}/profile?fixture=outage&invite=TESTfixt01`);
    browser("wait", "--text", "Your account didn’t load.");
    assert.equal(evaluate("location.pathname"), "/profile", "An outage must not sign the visitor out");
    assert.equal(evaluate("new URLSearchParams(location.search).get('invite')"), "TESTfixt01");

    browser("open", `${origin}/spaces?fixture=newcomer`);
    if (width === 390) {
      browser("wait", '[aria-label="Back to Browse"]');
      browser("click", '[aria-label="Back to Browse"]');
    }
    browser("wait", '[aria-label="Create space"]');
    browser("click", '[aria-label="Create space"]');
    browser("wait", "dialog[open]");
    assert.ok(evaluate("document.querySelector('dialog[open]').innerText.includes('it looks like')"));
    assert.equal(evaluate("document.querySelector('.join-invite-link button').disabled"), true);
    // The decorative hint must not intercept clicks or remain over a pasted link.
    browser("find", "label", "Invite link", "click");
    assert.equal(evaluate("document.activeElement.labels[0].textContent.trim()"), "Invite link");
    browser("find", "label", "Invite link", "fill", "caper.chat/invite/TESTfixt01");
    browser(
      "wait",
      "--fn",
      "!document.querySelector('dialog[open]').innerText.includes('it looks like') && !document.querySelector('.join-invite-link button').disabled",
    );
    browser("press", "Enter");
    browser("wait", "--url", "**/invite/TESTfixt01");
    assert.equal(evaluate("inviteFixture.posts"), 0, "Pasting a link must only open its preview");
    for (const mode of ["newcomer", "member", "guest", "legacy"]) {
      browser("open", `${origin}/invite/TESTfixt01?fixture=${mode}`);
      browser("wait", "--fn", '!!document.querySelector("main button")');
      assert.equal(evaluate("inviteFixture.posts"), 0, "Preview must not accept an invite");
      const action = mode === "member" ? "Open space" : mode === "guest" ? "Sign in to join" : "Join space";
      assert.ok(evaluate("document.querySelector('main button').textContent").includes(action));
      assert.equal(
        evaluate("document.querySelector('main').textContent.includes('YOU’RE ALREADY A MEMBER')"),
        mode === "member",
      );
      assert.equal(
        evaluate("document.querySelector('a[href^=\"caper://\"]').getAttribute('href')"),
        "caper://invite/TESTfixt01",
      );
      if (artifacts && mode !== "legacy") {
        evaluate(
          "document.fonts.ready.then(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))))",
        );
        browser("screenshot", join(artifacts, `invite-${mode}-${width}.png`));
      }
      browser("click", "main button");
      browser("wait", "--fn", mode === "guest" ? "location.pathname === '/login'" : "location.pathname === '/spaces'");
      assert.equal(evaluate("inviteFixture.posts"), mode === "guest" ? 0 : 1);
      assert.equal(
        evaluate("location.search"),
        mode === "guest" ? "?invite=TESTfixt01" : "?space=space0000001&channel=channel00001",
      );
    }
  }
  console.log(
    "PASS: desktop/narrow invite recovery through sign-in and profile setup, invalid/missing invite and outage controls, accessible invite entry, decorative hint click-through and dismissal, member Open space, newcomer Join space, signed-out sign-in, older API fallback, read-only previews, and landing-channel navigation.",
  );
} finally {
  try {
    browser("close");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
