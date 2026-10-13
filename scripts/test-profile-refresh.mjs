// Labelled loopback browser fixtures; no real accounts, media or shared-state writes.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const url = process.env.PROFILE_TEST_WEB_URL ?? "http://localhost:5174/spaces";
assert.ok(["localhost", "127.0.0.1"].includes(new URL(url).hostname));
const artifacts = process.env.PROFILE_TEST_ARTIFACTS && resolve(process.env.PROFILE_TEST_ARTIFACTS);
if (artifacts) mkdirSync(artifacts, { recursive: true });
const directory = mkdtempSync(join(tmpdir(), "caper-profile-"));
const init = join(directory, "fixture.js");
function fixture() {
  if (location.protocol === "about:") return;
  const original = { id: "owner1234567", username: "old_user", displayName: "TEST FIXTURE Old Name" };
  let account = { ...original };
  const other = { ...original, id: "other1234567", username: "other_user", owner: false };
  const spaces = [
    { id: "space1234567", name: "TEST FIXTURE · Studio", ownerId: account.id },
    { id: "space2345678", name: "TEST FIXTURE · Garden", ownerId: account.id },
  ];
  const channels = spaces.map((space, index) => ({
    id: `channel0000${index + 1}`,
    spaceId: space.id,
    name: "general",
    private: true,
    joined: true,
  }));
  const control = (window.profileFixture = { saves: 0, fail: false, hold: false, histories: 0 });
  const realFetch = window.fetch.bind(window);
  window.fetch = async (input, options = {}) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (!path.startsWith("/api/")) return realFetch(input, options);
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/account/profile") {
      if (control.hold)
        await new Promise((resolve) => {
          control.release = resolve;
        });
      if (control.fail) return Response.json({ error: "username unavailable" }, { status: 409 });
      account = { ...account, ...JSON.parse(options.body) };
      control.saves++;
      return Response.json(account);
    }
    if (path === "/api/spaces")
      return Response.json({
        spaces,
        invitations: [],
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path === "/api/spaces/unread") return Response.json({ spaces: [] });
    if (path === "/api/dms") return Response.json({ conversations: [] });
    if (path === "/api/people") return Response.json({ people: [other] });
    if (path === "/api/blocks") return Response.json({ blocks: [] });
    if (path === "/api/notifications/settings")
      return Response.json({ level: "all", mobile: "whenInactive", overrides: [] });
    if (path.endsWith("/invitations")) return Response.json({ members: [] });
    if (path.endsWith("/members")) return Response.json({ members: [{ ...original, owner: true }, other] });
    const space = spaces.find((space) => path === `/api/spaces/${space.id}`);
    // Retained/late metadata stays old on purpose: a saved account must win.
    if (space)
      return Response.json({
        space,
        channels: channels.filter((channel) => channel.spaceId === space.id),
        members: [{ ...original, owner: true }, other],
      });
    if (path === "/api/chat/session")
      return Response.json({
        token: "fixture-only",
        author: { id: account.id, name: account.displayName, isGuest: false },
      });
    const channel = channels.find((channel) => path === `/api/chat/channels/${channel.id}/messages`);
    if (channel) {
      control.histories++;
      return Response.json({
        space: spaces.find((space) => space.id === channel.spaceId),
        channel,
        cursor: "1",
        hasMore: false,
        messages: [
          {
            id: `${channel.id}msg`,
            channelId: channel.id,
            seq: "1",
            clientMessageId: "old-client",
            createdAt: "2026-10-11T12:00:00Z",
            author: { id: original.id, name: original.displayName, isGuest: false },
            content: { version: 1, type: "text", text: "TEST FIXTURE — retained message" },
          },
        ],
      });
    }
    if (path.endsWith("/media/status")) return Response.json({ enabled: false });
    return Response.json({ error: "TEST FIXTURE: endpoint disabled" }, { status: 503 });
  };
}
writeFileSync(init, `(${fixture.toString()})()`);
const args = ["--session", "profile-refresh", "--init-script", init];
const browser = (...command) => {
  const result = JSON.parse(
    execFileSync("agent-browser", [...args, ...command, "--json"], { encoding: "utf8", timeout: 60000 }),
  );
  assert.ok(result.success, result.error);
  return result.data;
};
const evaluate = (source) => browser("eval", source).result;
const wait = (source) => browser("wait", "--fn", source);
const click = (name) => browser("find", "role", "button", "click", "--name", name, "--exact");
const profileSaveSize = () =>
  evaluate(`(() => {
  const r = document.querySelector('.profile-dialog form button[type=submit]').getBoundingClientRect();
  return [r.width, r.height];
})()`);
try {
  browser("open", "about:blank");
  browser("set", "viewport", "1280", "900", "2");
  browser("open", url);
  for (const [width, height] of [
    [1280, 900],
    [390, 844],
    [390, 500],
  ]) {
    browser("set", "viewport", String(width), String(height), "2");
    browser("open", url);
    wait('!!document.querySelector(".space-menu summary") && !!document.querySelector(".chat-author-name")');
    if (width < 760) click("Back to Browse");
    wait('!!document.querySelector(".spaces-room.navigation-open") || innerWidth >= 760');
    wait('!document.querySelector(".spaces-room")?.hasAttribute("data-browse-slide")');
    click("Edit profile for TEST FIXTURE Old Name");
    wait('!!document.querySelector(".profile-dialog[open] form")');
    evaluate('document.querySelector(".profile-dialog form button[type=submit]").scrollIntoView({block:"nearest"})');
    const size = profileSaveSize();
    assert.ok(size[0] <= 160 && size[1] <= 48, "Profile save must be compact");
    assert.ok(size[1] >= 44, "Profile save must retain a touch-sized target");
    if (artifacts) browser("screenshot", `${artifacts}/profile-dialog-${width}-${height}.png`);
    evaluate("profileFixture.fail = true; profileFixture.hold = true");
    browser("click", ".profile-dialog form button[type=submit]");
    wait('document.querySelector(".profile-dialog form button[type=submit]").disabled && !!profileFixture.release');
    assert.deepEqual(profileSaveSize(), size, "Pending must not resize the profile action");
    if (artifacts) browser("screenshot", `${artifacts}/profile-dialog-saving-${width}-${height}.png`);
    evaluate("profileFixture.release(); profileFixture.hold = false");
    wait('document.querySelector(".profile-dialog [role=alert]")?.textContent.includes("already taken")');
    assert.deepEqual(profileSaveSize(), size, "Error must not resize the profile action");
    if (artifacts) browser("screenshot", `${artifacts}/profile-dialog-error-${width}-${height}.png`);
    browser("click", "#display-name");
    assert.ok(evaluate('!!document.querySelector(".profile-dialog[open]")'), "Inside clicks must not dismiss");
    // Releasing a drag that began inside must not discard the form.
    browser("mouse", "down", "left");
    browser("mouse", "move", "1", "1");
    browser("mouse", "up", "left");
    assert.ok(
      evaluate('!!document.querySelector(".profile-dialog[open]")'),
      "An inside-to-outside drag must not dismiss",
    );
    browser("mouse", "down", "left");
    browser("mouse", "up", "left");
    wait('!document.querySelector(".profile-dialog[open]")');
    assert.equal(evaluate("profileFixture.saves"), 0, "Backdrop dismissal must not save edits");
    click("Edit profile for TEST FIXTURE Old Name");
    browser("press", "Escape");
    wait('!document.querySelector(".profile-dialog[open]")');
  }
  console.log(
    "PASS: compact profile save, pending/error states, inside clicks, inside-to-outside drags, backdrop dismissal without saving, and Escape at 1280x900, 390x844 and 390x500.",
  );
  browser("set", "viewport", "1280", "900", "2");
  browser("open", url);
  wait('!!document.querySelector(".space-menu summary") && !!document.querySelector(".chat-author-name")');
  browser("fill", ".chat-composer textarea", "Keep this unsent draft");
  click("Edit profile for TEST FIXTURE Old Name");
  browser("fill", "#username", "new_user");
  browser("fill", "#display-name", "TEST FIXTURE New Name");
  evaluate("profileFixture.fail = true");
  browser("click", ".profile-dialog form button[type=submit]");
  wait('document.querySelector(".profile-dialog [role=alert]")?.textContent.includes("already taken")');
  assert.equal(evaluate("profileFixture.saves"), 0);
  evaluate("profileFixture.fail = false");
  browser("click", ".profile-dialog form button[type=submit]");
  wait('!document.querySelector(".profile-dialog[open]")');
  assert.equal(evaluate('document.querySelector(".chat-composer textarea").value'), "Keep this unsent draft");
  wait('document.querySelector(".chat-author-name")?.textContent.includes("TEST FIXTURE New Name")');
  assert.equal(evaluate("profileFixture.histories"), 1, "saving never reloads chat");
  for (const width of [1280, 390]) {
    browser("set", "viewport", String(width), "900", "2");
    if (width === 390) click("Back to Browse");
    wait('document.getAnimations().every(animation => animation.playState !== "running")');
    for (const [menu, item, dialog] of [
      ["space", "last-child", "Manage space"],
      ["channel", "nth-last-child(2)", "Channel settings"],
    ]) {
      browser("focus", `.${menu}-menu summary`);
      if (!evaluate(`document.querySelector(".${menu}-menu")?.open`)) browser("press", "Enter");
      wait(`document.querySelector(".${menu}-menu")?.open === true`);
      browser("focus", `.${menu}-menu .space-actions > button:${item}`);
      browser("press", "Enter");
      wait('document.querySelector(".member-manager")?.textContent.includes("@new_user")');
      const rows = evaluate('[...document.querySelectorAll(".member-manager li")].map(row => row.textContent)');
      assert.ok(
        rows.some((row) => row.includes("TEST FIXTURE New Name") && row.includes("@new_user")),
        "settings use the saved identity immediately",
      );
      assert.ok(
        rows.some((row) => row.includes("TEST FIXTURE Old Name") && row.includes("@other_user")),
        "same-name peer stays unchanged",
      );
      if (artifacts) browser("screenshot", `${artifacts}/profile-${menu}-settings-${width}.png`);
      click(`Close ${dialog}`);
    }
  }
  browser("set", "viewport", "1280", "900", "2");
  click("TEST FIXTURE · Garden");
  wait('document.querySelector(".space-menu")?.textContent.includes("Garden")');
  click("TEST FIXTURE · Studio");
  wait('document.querySelector(".space-menu")?.textContent.includes("Studio")');
  wait('document.querySelector(".chat-author-name")?.textContent.includes("TEST FIXTURE New Name")');
  browser("click", ".space-menu summary");
  click("Space settings");
  wait('document.querySelector(".member-manager")?.textContent.includes("@new_user")');
  console.log(
    "PASS: desktop/narrow space and channel settings and cached navigation show saved profile; same-name peer, failed save, history and draft stay correct.",
  );
} finally {
  browser("close");
  rmSync(directory, { recursive: true, force: true });
}
