// Disposable browser mocks. Never writes real accounts/spaces or calls an SFU.
// SPACES_TEST_WEB_URL=http://localhost:5174/spaces node scripts/test-space-invitations.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const url = process.env.SPACES_TEST_WEB_URL ?? "http://localhost:5174/spaces";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.SPACES_TEST_ARTIFACTS && resolve(process.env.SPACES_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(join(tmpdir(), "caper-invitations-"));
const init = join(directory, "fixture.js");
function fixture() {
  if (location.protocol === "about:") return;
  const params = new URL(location.href).searchParams;
  const recipient = params.has("recipient");
  const account = {
    id: recipient ? "member123456" : "owner1234567",
    username: recipient ? "member" : "owner",
    displayName: "TEST FIXTURE account",
  };
  const space = { id: "space1234567", name: "TEST FIXTURE · Studio", ownerId: "owner1234567" };
  const channels = params.has("links")
    ? ["general", "design", "private"].map((name, index) => ({
        id: `channel0000${index}`,
        spaceId: space.id,
        name,
        private: index === 2,
        joined: true,
      }))
    : [];
  const inviter = params.has("legacy")
    ? undefined
    : { username: "fixture_host", displayName: params.has("long") ? "A".repeat(64) : "TEST FIXTURE host" };
  const member = { id: "friend123456", username: "friend", displayName: "TEST FIXTURE friend", owner: false };
  const control = (window.inviteFixture = {
    requests: [],
    pending: [],
    accepted: params.has("links"),
    links: [],
    declined: false,
    revoked: false,
    fail: false,
  });
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (!path.startsWith("/api/")) return originalFetch(input, options);
    const method = options.method ?? "GET";
    control.requests.push({ path, method, body: options.body });
    const error = (status, message) => Response.json({ error: message }, { status });
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/dms") return Response.json({ conversations: [] });
    if (path === `/api/spaces/${space.id}/invite-links`) {
      if (method === "POST") {
        const body = JSON.parse(options.body);
        const link = {
          code: `FixtureCode${control.links.length}`,
          channel: channels.find((c) => c.id === body.channelId) ?? null,
          createdBy: account,
          uses: 0,
          maxUses: body.maxUses,
          expiresAt: null,
          createdAt: new Date().toISOString(),
        };
        control.links.push(link);
        return Response.json(link, { status: 201 });
      }
      return Response.json({ links: control.links });
    }
    if (path.endsWith("/messages")) {
      const channel = channels.find((c) => path.includes(c.id));
      return Response.json({ space, channel, messages: [], cursor: "0", hasMore: false });
    }
    if (path === "/api/spaces")
      return Response.json({
        spaces: !control.revoked && (!recipient || control.accepted) ? [space] : [],
        invitations: recipient && !control.accepted && !control.declined ? [{ ...space, inviter }] : [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path === `/api/spaces/${space.id}/invitation`) {
      if (control.fail) return error(503, "TEST FIXTURE: temporary failure");
      if (method === "DELETE") {
        control.declined = true;
        return new Response(null, { status: 204 });
      }
      control.accepted = true;
      return Response.json(space);
    }
    if (path === `/api/spaces/${space.id}/invitations`) return Response.json({ members: control.pending });
    if (path.startsWith(`/api/spaces/${space.id}/invitations/`)) {
      control.pending = [];
      return new Response(null, { status: 204 });
    }
    if (path === `/api/spaces/${space.id}/members` && method === "POST") {
      const { username } = JSON.parse(options.body);
      if (username === "missing") return error(404, "user not found");
      if (username === "owner") return error(409, "user already in space");
      if (username === "limited") return error(429, "too many invitation attempts; try again in 10 minutes");
      if (control.pending.length) return error(409, "user already invited");
      control.pending.push(member);
      return Response.json(member, { status: 201 });
    }
    if (path === `/api/spaces/${space.id}`) {
      if (control.revoked || (recipient && !control.accepted)) return error(404, "resource not found");
      return Response.json({ space, channels, members: [{ ...account, owner: !recipient }] });
    }
    return error(503, "TEST FIXTURE: disabled endpoint");
  };
}
writeFileSync(init, `(${fixture.toString()})()`);
const args = ["--session", "space-invites-test", "--init-script", init];
const browser = (...command) => {
  const result = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (expression) => browser("wait", "--fn", expression);
const screenshot = (name) => {
  if (!artifacts) return;
  evaluate(
    "Promise.all(document.getAnimations().filter(animation => animation.effect.getComputedTiming().iterations !== Infinity).map(animation => animation.finished))",
  );
  browser("screenshot", `${artifacts}/${name}.png`);
};
try {
  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    browser("open", `${url}?links&space=space1234567&channel=channel00000`);
    wait('!!document.querySelector(".space-menu")');
    if (width === 390) {
      browser("find", "role", "button", "click", "--name", "Back to Browse", "--exact");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room").hasAttribute("data-browse-slide")',
      );
    }
    browser("click", ".space-menu summary");
    screenshot(`invite-space-menu-${width}`);
    browser("find", "role", "button", "click", "--name", "Invite people…", "--exact");
    wait('!!document.querySelector(".invite-links select[id]")');
    assert.equal(
      evaluate('document.querySelector(".space-dialog h2").textContent'),
      "Invite people to TEST FIXTURE · Studio",
    );
    assert.equal(evaluate('document.querySelector(".invite-links select[id]").value'), "");
    screenshot(`invite-space-default-${width}`);
    browser("find", "role", "button", "click", "--name", "Create link", "--exact");
    wait("inviteFixture.links.length === 1");
    assert.equal(
      evaluate(
        'JSON.parse(inviteFixture.requests.find(r => r.method === "POST" && r.path.endsWith("/invite-links")).body).channelId',
      ),
      null,
    );
    browser("press", "Escape");
    browser("click", 'summary[aria-label="Manage design"]');
    screenshot(`invite-channel-menu-${width}`);
    browser("find", "role", "button", "click", "--name", "Invite people…", "--exact");
    wait('!!document.querySelector(".invite-links select[id]")');
    assert.equal(evaluate('document.querySelector(".invite-links select[id]").value'), "channel00001");
    assert.equal(
      evaluate('document.querySelector(".invite-links .channel-name-guidance").textContent'),
      "Anyone with this link can join TEST FIXTURE · Studio. They’ll start in #design.",
    );
    assert.equal(evaluate('document.querySelector(".channel-select[aria-current=page]").textContent'), "general");
    assert.equal(
      evaluate('document.querySelector(".invite-links select[id]").textContent.includes("#private")'),
      false,
    );
    screenshot(`invite-channel-preset-${width}`);
    browser("find", "role", "button", "click", "--name", "Create link", "--exact");
    wait("inviteFixture.links.length === 2");
    assert.equal(
      evaluate(
        'JSON.parse(inviteFixture.requests.filter(r => r.method === "POST" && r.path.endsWith("/invite-links")).at(-1).body).channelId',
      ),
      "channel00001",
    );
    browser("press", "Escape");
    browser("click", 'summary[aria-label="Manage private"]');
    assert.equal(
      evaluate('document.querySelector(".channel-menu[open]").textContent.includes("Invite people")'),
      false,
    );
    browser("press", "Escape");
    browser("click", ".direct-action");
    wait('!!document.querySelector(".space-dialog")');
    assert.equal(evaluate('document.querySelector(".space-dialog h2").textContent'), "New direct message");
    browser("press", "Escape");
    browser("click", ".space-menu summary");
    browser("find", "role", "button", "click", "--name", "Invite people…", "--exact");
    assert.equal(
      evaluate('document.querySelector(".invite-links select[id]").value'),
      "",
      "space entry resets the previous channel preset",
    );
    assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);

    browser("open", `${url}?links&recipient&space=space1234567&channel=channel00000`);
    wait('!!document.querySelector(".space-menu")');
    if (width === 390) {
      browser("find", "role", "button", "click", "--name", "Back to Browse", "--exact");
      wait(
        '!!document.querySelector(".spaces-room.navigation-open") && !document.querySelector(".spaces-room").hasAttribute("data-browse-slide")',
      );
    }
    browser("click", ".space-menu summary");
    assert.equal(evaluate('document.querySelector(".space-menu").textContent.includes("Invite people")'), false);
    browser("press", "Escape");
    browser("click", 'summary[aria-label="Manage design"]');
    assert.equal(
      evaluate('document.querySelector(".channel-menu[open]").textContent.includes("Invite people")'),
      false,
    );
  }
  console.log(
    "PASS: owner-only space/public-channel invites, clicked-channel payload, default reset, private-channel exclusion, DM footer; desktop + narrow Chromium.",
  );
  browser("set", "viewport", "1280", "900", "2");
  browser("open", url);
  wait('!!document.querySelector(".space-menu")');
  browser("click", ".space-menu summary");
  browser("find", "role", "button", "click", "--name", "Space settings", "--exact");
  wait('!!document.querySelector(".member-add")');
  browser("fill", "#member-username", "Fr IEND!!");
  assert.equal(evaluate('document.querySelector("#member-username").value'), "friend");
  browser("fill", "#member-username", "missing");
  browser("find", "role", "button", "click", "--name", "Invite", "--exact");
  wait('document.querySelector(".member-manager [role=alert]")?.textContent.includes("User not found")');
  screenshot("invite-user-not-found");
  browser("fill", "#member-username", "owner");
  browser("find", "role", "button", "click", "--name", "Invite", "--exact");
  wait('document.querySelector(".member-manager [role=alert]")?.textContent.includes("already in the space")');
  assert.equal(
    evaluate('inviteFixture.requests.filter(r => r.method === "POST").length'),
    1,
    "known members do not trigger an API request",
  );
  browser("fill", "#member-username", "friend");
  browser("find", "role", "button", "click", "--name", "Invite", "--exact");
  wait('document.querySelector(".member-manager:not(.invite-links)")?.textContent.includes("Invited")');
  assert.equal(
    evaluate('document.querySelector(".member-manager:not(.invite-links) > ul").children.length'),
    1,
    "invite does not enter active member list",
  );
  screenshot("invite-owner-pending");
  browser("fill", "#member-username", "friend");
  browser("find", "role", "button", "click", "--name", "Invite", "--exact");
  wait('document.querySelector(".member-manager [role=alert]")?.textContent.includes("pending invitation")');
  browser("find", "role", "button", "click", "--name", "Cancel invite");
  wait("inviteFixture.pending.length === 0");
  browser("fill", "#member-username", "limited");
  browser("find", "role", "button", "click", "--name", "Invite", "--exact");
  wait('document.querySelector(".member-manager [role=alert]")?.textContent.includes("10 minutes")');

  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    browser("open", `${url}?recipient`);
    wait('!!document.querySelector(".pending-space-invite")');
    browser("click", ".pending-space-invite");
    wait('!!document.querySelector(".invitation-consent")');
    assert.equal(evaluate('document.querySelector(".space-dialog h2").textContent'), "You’re invited!");
    wait('document.querySelector(".space-dialog > header h2 img")?.naturalWidth === 64');
    evaluate(
      'Promise.all(document.querySelector(".space-dialog").getAnimations().map(animation => animation.finished))',
    );
    assert.deepEqual(
      evaluate(
        '(() => { const image = document.querySelector(".space-dialog > header h2 img"); const rect = image.getBoundingClientRect(); return [image.getAttribute("src"), image.alt, rect.width, rect.height]; })()',
      ),
      ["/images/invitation/1f4e8.png", "", 32, 32],
    );
    assert.ok(
      evaluate(
        '(() => { const heading = document.querySelector(".space-dialog h2"); const image = heading.querySelector("img").getBoundingClientRect(); const range = document.createRange(); range.selectNodeContents(heading.lastChild); const text = range.getBoundingClientRect(); return image.right < text.left && Math.abs(image.y + image.height / 2 - text.y - text.height / 2) < 4; })()',
      ),
      "envelope sits beside the title on the same line",
    );
    assert.equal(
      evaluate('document.querySelectorAll(".invitation-consent img").length'),
      0,
      "no duplicate artwork in the body",
    );
    assert.equal(
      evaluate('document.querySelector(".invitation-consent h3").textContent'),
      "Join TEST FIXTURE · Studio?",
    );
    assert.deepEqual(evaluate('[...document.querySelectorAll(".invitation-consent > p")].map(p => p.textContent)'), [
      "TEST FIXTURE host (@fixture_host) invited you.",
    ]);
    assert.equal(
      evaluate(
        'inviteFixture.requests.some(r => r.path === "/api/spaces/space1234567" || /channels|messages|presence|media/.test(r.path))',
      ),
      false,
      "no private data is requested before acceptance",
    );
    assert.equal(evaluate("document.activeElement.textContent"), "Decline");
    assert.equal(evaluate('document.querySelector(".invitation-shell").inert'), true);
    assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
    screenshot(`invitation-consent-${width}`);
    if (width === 1280) {
      evaluate("inviteFixture.fail = true");
      browser("find", "role", "button", "click", "--name", "Accept invitation");
      wait('!!document.querySelector(".invitation-consent [role=alert]")');
      assert.equal(evaluate("inviteFixture.accepted"), false);
      evaluate("inviteFixture.fail = false");
      browser("find", "role", "button", "click", "--name", "Decline", "--exact");
      wait('!document.querySelector(".invitation-consent")');
      assert.equal(evaluate('inviteFixture.requests.some(r => r.path === "/api/spaces/space1234567")'), false);
    } else {
      browser("find", "role", "button", "click", "--name", "Accept invitation");
      wait('inviteFixture.accepted && !!document.querySelector(".empty-channel")');
      assert.ok(evaluate('inviteFixture.requests.some(r => r.path === "/api/spaces/space1234567")'));
      evaluate('inviteFixture.revoked = true; window.dispatchEvent(new Event("focus"))');
      wait('document.querySelector(".spaces-empty")?.textContent.includes("This space is no longer available")');
      assert.equal(
        evaluate('document.querySelector(".space-rail") !== null'),
        false,
        "revocation removes the rail entry",
      );
      screenshot("space-revoked-390");
    }
  }
  for (const variant of ["long", "legacy"]) {
    browser("open", `${url}?recipient&${variant}`);
    wait('!!document.querySelector(".pending-space-invite")');
    browser("click", ".pending-space-invite");
    wait('!!document.querySelector(".invitation-consent")');
    assert.equal(
      evaluate('document.querySelector(".invitation-consent h3").textContent'),
      "Join TEST FIXTURE · Studio?",
    );
    assert.deepEqual(
      evaluate('[...document.querySelectorAll(".invitation-consent > p")].map(p => p.textContent)'),
      variant === "long" ? [`${"A".repeat(64)} (@fixture_host) invited you.`] : [],
    );
    assert.equal(evaluate("document.documentElement.scrollWidth > innerWidth"), false);
    assert.equal(
      evaluate(
        'inviteFixture.requests.some(r => r.path === "/api/spaces/space1234567" || /channels|messages|presence|media/.test(r.path))',
      ),
      false,
    );
    screenshot(`invitation-consent-${variant}-390`);
  }
  console.log(
    "PASS: concise invitation name/inviter copy, shared artwork/title, normalized usernames, missing/duplicate/member/rate-limit errors, cancel, long names/legacy metadata, consent privacy, retry, decline, acceptance and revocation; desktop + narrow Chromium.",
  );
} finally {
  browser("close");
  rmSync(directory, { recursive: true, force: true });
}
