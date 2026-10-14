import assert from "node:assert/strict";
import { test, vi } from "vitest";
import { androidAssetLinks, appleAppSiteAssociation } from "./app-links.ts";
import {
  appInviteUrl,
  createInviteLink,
  ensureDefaultInviteLink,
  DEFAULT_INVITE_EXPIRY,
  DEFAULT_INVITE_MAX_USES,
  inviteExpirySeconds,
  inviteMaxUses,
  inviteCode,
  inviteExpiry,
  inviteLinkSummary,
  invitedSpacePath,
  inviteFromSearch,
  joinInvite,
  previewInvite,
  signedInPath,
  SpacesApiError,
} from "../spaces/client.ts";

test("invite codes come from full, scheme-less or bare pasted links", () => {
  for (const input of [
    "AbCdE12345",
    "  AbCdE12345 ",
    "https://caper.chat/invite/AbCdE12345",
    "https://caper.chat/invite/AbCdE12345/",
    "http://localhost:3000/invite/AbCdE12345?utm=x#top",
    "caper.chat/invite/AbCdE12345",
    "/invite/AbCdE12345",
  ])
    assert.equal(inviteCode(input), "AbCdE12345", input);
  for (const input of [
    "",
    "AbCdE1234",
    "AbCdE123456",
    "https://caper.chat/invite/AbCdE1234!",
    "https://caper.chat/spaces?space=AbCdE12345",
    "https://caper.chat/invite/AbCdE12345/extra",
    "javascript:alert(1)//invite/AbCdE12345x",
  ])
    assert.equal(inviteCode(input), undefined, input);
});

test("sign-in returns to a pending invite only for valid codes", () => {
  assert.equal(inviteFromSearch("?invite=AbCdE12345"), "AbCdE12345");
  assert.equal(inviteFromSearch("?invite=//evil.example"), undefined);
  assert.equal(signedInPath({ username: "jose" }, "?invite=AbCdE12345"), "/invite/AbCdE12345");
  assert.equal(signedInPath({ username: null }, "?invite=AbCdE12345"), "/profile?invite=AbCdE12345");
  assert.equal(signedInPath({ username: "jose" }, ""), "/spaces");
  assert.equal(signedInPath({ username: null }, "?invite=bad"), "/profile");
  assert.equal(invitedSpacePath("space1234567", "chan12345678"), "/spaces?space=space1234567&channel=chan12345678");
  assert.equal(invitedSpacePath("space1234567", null), "/spaces?space=space1234567");
});

test("invite link summaries read like Discord's", () => {
  const now = Date.parse("2026-10-11T00:00:00Z");
  const link = {
    code: "AbCdE12345",
    channel: { id: "chan12345678", name: "feedback" },
    createdBy: { username: "jose", displayName: "Jose" },
    uses: 3,
    maxUses: 10,
    expiresAt: "2026-10-17T00:00:00Z",
    createdAt: "2026-10-10T00:00:00Z",
  };
  assert.equal(inviteLinkSummary(link, now), "#feedback · 3 of 10 uses · Expires in 6 days");
  assert.equal(
    inviteLinkSummary({ ...link, channel: null, uses: 1, maxUses: null, expiresAt: null }, now),
    "1 use · Never expires",
  );
  assert.equal(
    inviteLinkSummary({ ...link, uses: 1_234, maxUses: 10_000, expiresAt: "2026-11-10T00:00:00Z" }, now),
    "#feedback · 1,234 of 10,000 uses · Expires in 30 days",
  );
  assert.equal(inviteExpiry("2026-10-11T00:30:00Z", now), "Expires in 30 minutes");
  assert.equal(inviteExpiry("2026-10-11T01:00:00Z", now), "Expires in 1 hour");
  assert.equal(inviteExpiry("2026-10-10T23:00:00Z", now), "Expires in 1 minute");
});

test("invite requests use the documented endpoints and reject malformed codes locally", async () => {
  const calls: Array<{ path: string; method?: string; body?: string }> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ path: String(input), method: init?.method, body: init?.body as string | undefined });
    return new Response(JSON.stringify({}), { status: 200, headers: { "content-type": "application/json" } });
  });
  await createInviteLink("space1234567", { channelId: null, maxAge: 604800, maxUses: null });
  await ensureDefaultInviteLink("space1234567");
  await previewInvite("AbCdE12345");
  await joinInvite("AbCdE12345");
  assert.deepEqual(calls, [
    {
      path: "/api/spaces/space1234567/invite-links",
      method: "POST",
      body: JSON.stringify({ channelId: null, maxAge: 604800, maxUses: null }),
    },
    { path: "/api/spaces/space1234567/invite-links/default", method: "POST", body: undefined },
    { path: "/api/invites/AbCdE12345", method: undefined, body: undefined },
    { path: "/api/invites/AbCdE12345/join", method: "POST", body: undefined },
  ]);
  await assert.rejects(() => previewInvite("../x"), SpacesApiError);
});

test("native apps verify caper.chat invite links and open caper:// links", async () => {
  const apple = await appleAppSiteAssociation().json();
  assert.deepEqual(apple.applinks.details[0].appIDs, ["NAUU43LATL.chat.caper.ios"]);
  assert.deepEqual(apple.applinks.details[0].components[0]["/"], "/invite/*");
  const android = await androidAssetLinks().json();
  assert.equal(android[0].target.package_name, "chat.caper.android");
  for (const fingerprint of android[0].target.sha256_cert_fingerprints)
    assert.match(fingerprint, /^([0-9A-F]{2}:){31}[0-9A-F]{2}$/);
  assert.equal(appInviteUrl("AbCdE12345"), "caper://invite/AbCdE12345");
  assert.equal(inviteCode("caper://invite/AbCdE12345"), "AbCdE12345");
});

test("expiry and max uses are free-form within the API's bounds", () => {
  assert.deepEqual(inviteExpirySeconds(DEFAULT_INVITE_EXPIRY.amount, DEFAULT_INVITE_EXPIRY.unit), { value: 1_382_400 });
  assert.deepEqual(inviteMaxUses(DEFAULT_INVITE_MAX_USES), { value: 100 });
  assert.deepEqual(inviteExpirySeconds("7", "days"), { value: 604_800 });
  assert.deepEqual(inviteExpirySeconds(" 90 ", "minutes"), { value: 5_400 });
  assert.deepEqual(inviteExpirySeconds("1", "minutes"), { value: 60 });
  assert.deepEqual(inviteExpirySeconds("365", "days"), { value: 31_536_000 });
  assert.deepEqual(inviteExpirySeconds("", "never"), { value: null });
  for (const [amount, unit] of [
    ["0", "minutes"],
    ["", "hours"],
    ["366", "days"],
    ["8761", "hours"],
    ["1.5", "days"],
    ["-1", "days"],
  ] as const)
    assert.ok(inviteExpirySeconds(amount, unit).error, `${amount} ${unit}`);
  assert.deepEqual(inviteMaxUses(""), { value: null });
  assert.deepEqual(inviteMaxUses(" 7 "), { value: 7 });
  assert.deepEqual(inviteMaxUses("10000"), { value: 10_000 });
  for (const text of ["0", "10001", "2.5", "abc", "-3"]) assert.ok(inviteMaxUses(text).error, text);
});
