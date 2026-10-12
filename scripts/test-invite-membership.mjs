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
  const state = (window.inviteFixture = { posts: 0 });
  const original = window.fetch.bind(window);
  window.fetch = async (input, options) => {
    const path = new URL(input instanceof Request ? input.url : String(input), location.href).pathname;
    if (!path.startsWith("/api/")) return original(input, options);
    if (path === "/api/account/me")
      return mode === "guest" ? Response.json({ error: "unauthorized" }, { status: 401 }) : Response.json(account);
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
    if (path === "/api/spaces") return Response.json({ spaces: [space], invitations: [] });
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
      if (mode !== "guest") assert.equal(evaluate("location.search"), "?space=space0000001&channel=channel00001");
    }
  }
  console.log(
    "PASS: desktop/narrow member Open space, newcomer Join space, signed-out sign-in, older API fallback, read-only previews, and landing-channel navigation.",
  );
} finally {
  try {
    browser("close");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
