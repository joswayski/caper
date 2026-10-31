// Custom profile pictures and space icons in the real web UI, against explicitly
// labelled in-page API/gateway mocks and a fake CDN/storage served by Playwright.
// No real accounts, uploads, CDN or SFU calls.
//
//   PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs \
//   PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers \
//   node scripts/test-pictures.mjs http://localhost:5174 [screenshot-directory]
// CHROMIUM_PATH can select an existing Chrome executable instead of Playwright's browser.
// PICTURE_VIEWER_ONLY=1 runs viewers; PICTURE_GALLERY_ONLY=1 also runs drafts/galleries,
// skipping the broader rendering/upload/space-icon suite.
import assert from "node:assert/strict";
import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { crc32, deflateSync } from "node:zlib";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const origin = new URL(process.argv[2] ?? "http://localhost:5174");
assert.ok(["localhost", "127.0.0.1"].includes(origin.hostname));
const artifacts = process.argv[3] && resolve(process.argv[3]);
if (artifacts) mkdirSync(artifacts, { recursive: true });

// ---- Generated test images ----------------------------------------------------

const palette = {
  green: [0x63, 0x7a, 0x43],
  eggshell: [0xf1, 0xed, 0xe4],
  terracotta: [0xb6, 0x4d, 0x32],
  blackout: [0x0c, 0x0d, 0x0f],
};

/** An animated GIF (4 colours, looping). LZW codes stay 3 bits by clearing every two pixels. */
function gif(width, height, colors, frames, delay = 25) {
  const bytes = [];
  const u16 = (value) => bytes.push(value & 0xff, value >> 8);
  bytes.push(...Buffer.from("GIF89a"));
  u16(width);
  u16(height);
  bytes.push(0xf1, 0, 0);
  for (const color of colors) bytes.push(...color);
  bytes.push(0x21, 0xff, 11, ...Buffer.from("NETSCAPE2.0"), 3, 1, 0, 0, 0);
  for (const frame of frames) {
    bytes.push(0x21, 0xf9, 4, 0x04, delay & 0xff, delay >> 8, 0, 0);
    bytes.push(0x2c);
    u16(0);
    u16(0);
    u16(width);
    u16(height);
    bytes.push(0, 2);
    const codes = [];
    let count = 0;
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        if (count++ % 2 === 0) codes.push(4);
        codes.push(frame(x, y));
      }
    codes.push(5);
    const data = [];
    let accumulator = 0;
    let bits = 0;
    for (const code of codes) {
      accumulator |= code << bits;
      bits += 3;
      while (bits >= 8) {
        data.push(accumulator & 0xff);
        accumulator >>= 8;
        bits -= 8;
      }
    }
    if (bits) data.push(accumulator & 0xff);
    for (let index = 0; index < data.length; index += 255) {
      const block = data.slice(index, index + 255);
      bytes.push(block.length, ...block);
    }
    bytes.push(0);
  }
  bytes.push(0x3b);
  return Buffer.from(bytes);
}

/** A block that moves around the four quadrants, one per frame. */
const quadrantFrames = (size, background, block) =>
  [0, 1, 2, 3].map(
    (quadrant) => (x, y) =>
      Math.floor(x / (size / 2)) === quadrant % 2 && Math.floor(y / (size / 2)) === Math.floor(quadrant / 2)
        ? block
        : background,
  );

function png(width, height, pixel) {
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) raw.set(pixel(x, y), y * (width * 3 + 1) + 1 + x * 3);
  const chunk = (type, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, crc]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const colors = [palette.green, palette.eggshell, palette.terracotta, palette.blackout];
const animatedAvatar = gif(64, 64, colors, quadrantFrames(64, 0, 2));
const animatedIcon = gif(48, 48, colors, quadrantFrames(48, 2, 1));
const stillAvatar = png(96, 96, (x, y) => (((x >> 4) + (y >> 4)) % 2 ? palette.eggshell : palette.green));
const stillIcon = png(64, 64, (x, y) => (Math.hypot(x - 32, y - 32) < 20 ? palette.eggshell : palette.blackout));
// A landscape photo-like PNG: the client must crop it square and re-encode it.
const landscape = png(800, 600, (x, y) => [Math.round((x / 800) * 255), Math.round((y / 600) * 255), 120]);
// Over 5 MB and 1024 px wide (this writer barely compresses): shrunk, never refused.
const oversizedGif = gif(
  1200,
  800,
  colors,
  quadrantFrames(1200, 0, 2).concat(quadrantFrames(1200, 1, 3), quadrantFrames(1200, 2, 0)),
);

// ---- In-page API and gateway mock ------------------------------------------------

function fixture() {
  if (location.protocol === "about:") return;
  const cdn = "https://cdn.test/assets";
  const url = (asset, signature = "a") => `${cdn}/${asset}?exp=1792000000&sig=${signature}`;
  const account = {
    id: "owner1234567",
    username: "fixture_owner",
    displayName: "Owner (test)",
    avatarId: 0,
  };
  const picture = (avatarAsset) => ({ avatarAsset, avatarUrl: url(avatarAsset) });
  const others = [
    {
      id: "member000001",
      username: "maya",
      displayName: "Maya (animated GIF)",
      avatarId: 31,
      ...picture("gifpicture01"),
    },
    { id: "member000002", username: "june", displayName: "June (still)", avatarId: 32, ...picture("stillpictur1") },
    {
      id: "member000003",
      username: "theo",
      displayName: "Theo (broken URL)",
      avatarId: 799,
      ...picture("brokenpictr1"),
    },
    { id: "member000004", username: "avery", displayName: "Avery (mascot)", avatarId: 143 },
  ];
  const control = (window.pictureFixture = {
    reservations: [],
    completes: 0,
    completions: [],
    writes: [],
    requests: [],
    profileSaves: [],
    removals: 0,
    failNext: undefined,
    history: [],
    selections: [],
  });
  control.savedAccount = () => ({ ...account });
  // The server's own copy follows a profile change made "elsewhere" (as the real API would).
  control.updateAccount = (fields) => Object.assign(account, fields);
  const members = () => [{ ...account, owner: true }, ...others.map((member) => ({ ...member, owner: false }))];
  const spaces = [
    {
      id: "space1234567",
      name: "TEST FIXTURE Studio",
      ownerId: account.id,
      iconAsset: "spaceicon001",
      iconUrl: url("spaceicon001"),
    },
    { id: "space2345678", name: "Garden (no icon)", ownerId: account.id },
  ];
  const invitations = [
    {
      id: "space3456789",
      name: "Invited (icon)",
      ownerId: others[0].id,
      iconAsset: "inviteicon01",
      iconUrl: url("inviteicon01"),
      inviter: { username: "maya", displayName: "Maya (animated GIF)" },
    },
  ];
  const channel = { id: "channel12345", spaceId: spaces[0].id, name: "general", private: false, joined: true };
  const author = (person) => ({
    id: person.id,
    name: person.displayName,
    isGuest: false,
    avatarId: person.avatarId,
    ...(person.avatarAsset ? { avatarAsset: person.avatarAsset, avatarUrl: person.avatarUrl } : {}),
  });
  const texts = [
    "Explicit test fixture: these are not real accounts or messages.",
    "My picture is an animated GIF.",
    "Mine is a still picture.",
    "Mine points at a picture that fails to load, so my mascot shows.",
    "I have no custom picture: my saved mascot.",
  ];
  const history = () => ({
    space: { id: spaces[0].id, name: spaces[0].name },
    channel,
    cursor: "5",
    hasMore: false,
    messages: [account, ...others].map((person, index) => ({
      id: `message${index}`,
      clientMessageId: `client${index}`,
      channelId: channel.id,
      seq: String(index + 1),
      author: author(person),
      content: { version: 1, type: "text", text: texts[index] },
      createdAt: "2026-10-14T12:00:00Z",
      // June replies to Maya: a nested author that live updates must reach too.
      ...(index === 2
        ? { replyTo: { id: "message1", author: author(others[0]), text: texts[1], hasAttachments: false } }
        : {}),
    })),
  });
  const spaceSummary = (space) => {
    const { id, name, ownerId, iconAsset, iconUrl } = space;
    return { id, name, ownerId, ...(iconAsset ? { iconAsset, iconUrl } : {}) };
  };
  const original = window.fetch.bind(window);
  let uploads = 0;
  window.fetch = async (input, options = {}) => {
    const requestUrl = new URL(typeof input === "string" ? input : input.url, location.href);
    const path = requestUrl.pathname;
    const method = (options.method ?? "GET").toUpperCase();
    if (!path.startsWith("/api/")) return original(input, options);
    control.requests.push(`${method} ${path}${requestUrl.search}`);
    // Background chat session/read acknowledgements are not profile writes.
    if (method !== "GET" && /^\/api\/(account|assets)(\/|$)/.test(path))
      control.writes.push(`${method} ${path}${requestUrl.search}`);
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/account/profile" && method === "POST") {
      const body = JSON.parse(options.body);
      control.profileSaves.push(body);
      if (control.holdProfile)
        await new Promise((resolve) => {
          control.finishProfile = resolve;
        });
      if (control.failProfile) {
        const failure = control.failProfile;
        control.failProfile = undefined;
        return Response.json(failure.body, { status: failure.status });
      }
      if (
        body.avatar &&
        (body.avatar.expectedAvatarId !== (account.avatarId ?? null) ||
          body.avatar.expectedAvatarAsset !== (account.avatarAsset ?? null))
      )
        return Response.json({ error: "Profile changed", code: "profile_changed" }, { status: 409 });
      if (body.avatar?.avatarAsset && !control.history.includes(body.avatar.avatarAsset))
        return Response.json({ error: "Profile picture is unavailable." }, { status: 404 });
      Object.assign(account, { username: body.username, displayName: body.displayName });
      if (body.avatar) {
        account.avatarId = body.avatar.avatarId;
        delete account.avatarAsset;
        delete account.avatarUrl;
        if (body.avatar.avatarAsset) Object.assign(account, picture(body.avatar.avatarAsset));
      }
      return Response.json(account);
    }
    if (path === "/api/account/avatar" && method === "DELETE") {
      control.removals++;
      delete account.avatarAsset;
      delete account.avatarUrl;
      return Response.json(account);
    }
    if (path === "/api/account/avatar/history") {
      if (control.holdHistory)
        await new Promise((resolve) => {
          control.finishHistory = resolve;
        });
      if (control.failHistory) return Response.json({ error: "Unavailable" }, { status: 503 });
      const cursor = requestUrl.searchParams.get("cursor");
      const start = cursor ? control.history.indexOf(cursor) + 1 : 0;
      const ids = control.history.slice(start, start + (control.pageSize ?? 40));
      return Response.json({
        pictures: ids.map(picture),
        nextCursor: start + ids.length < control.history.length ? ids.at(-1) : null,
      });
    }
    if (path === "/api/account/avatar/select" && method === "POST") {
      const body = JSON.parse(options.body);
      control.selections.push(body);
      if (control.holdSelect)
        await new Promise((resolve) => {
          control.finishSelect = resolve;
        });
      if (control.failSelect || body.expectedAvatarAsset !== (account.avatarAsset ?? null)) {
        control.failSelect = false;
        return Response.json({ error: "Your profile picture changed. Refresh and try again." }, { status: 409 });
      }
      if (!control.history.includes(body.avatarAsset))
        return Response.json({ error: "Profile picture is unavailable." }, { status: 404 });
      Object.assign(account, picture(body.avatarAsset));
      return Response.json(account);
    }
    if (path === "/api/assets" && method === "POST") {
      const body = JSON.parse(options.body);
      control.reservations.push(body);
      if (control.failNext) {
        const failure = control.failNext;
        control.failNext = undefined;
        return Response.json(failure.body, { status: failure.status });
      }
      const id = `upload${String(++uploads).padStart(6, "0")}`;
      return Response.json(
        {
          id,
          kind: "image",
          upload: {
            method: "PUT",
            url: `https://r2.test/put/${id}`,
            headers: { "content-type": body.contentType, "content-disposition": "inline" },
          },
          purpose: body.purpose,
          spaceId: body.spaceId,
        },
        { status: 201 },
      );
    }
    const complete = /^\/api\/assets\/(upload\d+)\/complete$/.exec(path);
    if (complete) {
      control.completes++;
      control.completions.push(`${path}${requestUrl.search}`);
      const reservation = control.reservations.at(-1);
      const id = complete[1];
      if (reservation.purpose === "avatar") {
        control.history.unshift(id);
        if (requestUrl.searchParams.get("draft") === "true") return Response.json({ id, kind: "image" });
        Object.assign(account, { avatarAsset: id, avatarUrl: url(id, "b") });
        return Response.json({ id, kind: "image", account });
      }
      const space = spaces.find((item) => item.id === reservation.spaceId);
      Object.assign(space, { iconAsset: id, iconUrl: url(id, "b") });
      return Response.json({ id, kind: "image", space: spaceSummary(space) });
    }
    if (/^\/api\/assets\/upload\d+\/cancel$/.test(path)) return new Response(null, { status: 204 });
    const icon = /^\/api\/spaces\/(\w+)\/icon$/.exec(path);
    if (icon && method === "DELETE") {
      const space = spaces.find((item) => item.id === icon[1]);
      delete space.iconAsset;
      delete space.iconUrl;
      return Response.json(spaceSummary(space));
    }
    if (path === "/api/assets/usage") return Response.json({ usedBytes: 0, limitBytes: 1 });
    if (path === "/api/invites/FIXTURE123")
      return Response.json({
        code: "FIXTURE123",
        space: { name: "Invited (icon)", iconAsset: "inviteicon01", iconUrl: url("inviteicon01") },
        channel: { name: "general" },
        inviter: { username: "maya", displayName: "Maya (animated GIF)", avatarId: 31, ...picture("gifpicture01") },
        memberCount: 5,
        expiresAt: null,
      });
    if (path === "/api/dms") return Response.json({ conversations: [] });
    if (path === "/api/people") return Response.json({ people: others });
    if (path === "/api/blocks") return Response.json({ blocks: [] });
    if (path === "/api/account/privacy") return Response.json({ directMessages: "anyone" });
    if (path === "/api/notifications/settings")
      return Response.json({ level: "all", mobile: "whenInactive", overrides: [] });
    if (path === "/api/spaces/unread") return Response.json({ spaces: [] });
    if (path === "/api/spaces")
      return Response.json({
        spaces: spaces.map(spaceSummary),
        invitations,
        limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
      });
    if (path.endsWith("/invitations")) return Response.json({ members: [] });
    if (path.endsWith("/invite-links")) return Response.json({ links: [] });
    if (path.endsWith("/members")) return Response.json({ members: members() });
    const space = spaces.find((item) => path === `/api/spaces/${item.id}`);
    if (space)
      return Response.json({
        space: spaceSummary(space),
        channels: space.id === channel.spaceId ? [channel] : [],
        members: members(),
      });
    if (path.endsWith("/status")) return Response.json({ enabled: true });
    if (path.endsWith("/session")) return Response.json({ token: "fixture-only", author: author(account) });
    if (path.endsWith("/messages")) return Response.json(history());
    return Response.json({ error: `TEST FIXTURE: unexpected request ${method} ${path}` }, { status: 400 });
  };
  const profileSockets = new Set();
  window.WebSocket = class extends EventTarget {
    profileIds = new Set();
    constructor() {
      super();
      queueMicrotask(() => this.frame({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    frame(frame) {
      this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(frame) }));
    }
    send(raw) {
      const request = JSON.parse(raw);
      if (request.type === "heartbeat") this.frame({ type: "heartbeat" });
      if (request.type === "unsubscribe") this.profileIds.delete(request.id);
      if (request.type === "subscribe" && request.kind === "profiles") {
        // Live profile/space updates: no snapshot, and events only when the test pushes them.
        // Both profile rendering and notification retractions subscribe to this
        // feed. Fan out like the gateway, rather than replacing its first listener.
        control.profilesFrame = request;
        profileSockets.add(this);
        this.profileIds.add(request.id);
        control.pushLive = (event) => {
          for (const socket of profileSockets)
            for (const id of socket.profileIds) socket.frame({ type: "event", id, event });
        };
        this.frame({ type: "subscribed", id: request.id });
        return;
      }
      if (request.type === "subscribe") {
        const event =
          request.kind === "chat" || request.kind === "notifications"
            ? { type: "ready", cursor: "5" }
            : request.kind === "presence"
              ? {
                  type: "snapshot",
                  members: (request.allMembers ? members().map((m) => m.id) : request.userIds).map((userId) => ({
                    userId,
                    status: "online",
                  })),
                }
              : {
                  type: "snapshot",
                  participants: [account, others[0], others[1]].map((person, index) => ({
                    id: `call-session-${index}`,
                    name: person.displayName,
                    avatarId: person.avatarId,
                    ...(person.avatarAsset ? { avatarAsset: person.avatarAsset, avatarUrl: person.avatarUrl } : {}),
                    muted: false,
                    deafened: false,
                  })),
                  revision: 1,
                };
        this.frame({ type: "event", id: request.id, event });
        this.frame({ type: "subscribed", id: request.id });
      }
      if (request.type === "command")
        this.frame({
          type: "result",
          id: request.id,
          status: 200,
          body: request.operation === "status" ? { enabled: true } : { token: "fixture-only", author: author(account) },
        });
    }
    close() {
      profileSockets.delete(this);
    }
  };
}

// ---- Fake CDN and storage --------------------------------------------------------

const stored = new Map([
  ["gifpicture01", { body: animatedAvatar, type: "image/gif" }],
  ["stillpictur1", { body: stillAvatar, type: "image/png" }],
  ["spaceicon001", { body: animatedIcon, type: "image/gif" }],
  ["inviteicon01", { body: stillIcon, type: "image/png" }],
]);
const cors = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "PUT",
  "access-control-allow-headers": "content-type, content-disposition",
};
async function context(browser, options) {
  const created = await browser.newContext({ deviceScaleFactor: 2, ...options });
  // Real image preparation/rendering shares the orb with native builds.
  created.setDefaultTimeout(60_000);
  await created.addInitScript(`(${fixture.toString()})();`);
  await created.route("https://cdn.test/assets/**", (route) => {
    const asset = new URL(route.request().url()).pathname.split("/").pop();
    const file = stored.get(asset);
    return file
      ? route.fulfill({
          status: 200,
          body: file.body,
          contentType: file.type,
          headers: { "cache-control": "max-age=3600", "access-control-allow-origin": "*" },
        })
      : route.fulfill({ status: 404, body: "TEST FIXTURE: missing picture" });
  });
  await created.route("https://r2.test/put/**", (route) => {
    const request = route.request();
    if (request.method() === "OPTIONS") return route.fulfill({ status: 204, headers: cors });
    const asset = new URL(request.url()).pathname.split("/").pop();
    stored.set(asset, { body: request.postDataBuffer(), type: request.headers()["content-type"] });
    return route.fulfill({ status: 200, headers: cors, body: "" });
  });
  return created;
}

const shot = async (target, name) => {
  if (artifacts) await target.screenshot({ path: join(artifacts, name), animations: "disabled" });
};
const loaded = (selector) =>
  `[...document.querySelectorAll(${JSON.stringify(selector)})].length > 0 && [...document.querySelectorAll(${JSON.stringify(selector)})].every((element) => element.querySelector(".caper-picture") && element.querySelector(".caper-picture").dataset.state !== "loading")`;
const ownAvatars =
  '.account-avatar [data-avatar-id], .direct-self [data-avatar-id], .chat-avatar[aria-label="Open profile for Owner (test)"] [data-avatar-id]';

/** Custom pictures are rounded squares: they take their avatar container's 30% corners, never a circle. */
async function checkSquares(target, label) {
  const shapes = await target.evaluate(() =>
    [...document.querySelectorAll("[data-avatar-asset] > .caper-picture")].map((picture) => ({
      where: picture.closest("[class]:not(.caper-picture):not([data-avatar-asset])")?.className,
      picture: getComputedStyle(picture).borderTopLeftRadius,
      avatar: getComputedStyle(picture.parentElement).borderTopLeftRadius,
      image: getComputedStyle(picture.querySelector("img")).borderTopLeftRadius,
    })),
  );
  assert.ok(shapes.length > 0, `${label}: pictures to check`);
  for (const shape of shapes) {
    assert.equal(shape.picture, "30%", `${label}: ${shape.where} picture`);
    assert.equal(shape.avatar, "30%", `${label}: ${shape.where} avatar`);
    assert.equal(shape.image, "0px", `${label}: ${shape.where} image`);
  }
}

/**
 * People in voice in a room you are not in show as overlapping faces (its row's live card,
 * or the phone strip). Picture faces are clipped rounded squares, and the ring that
 * separates them from their neighbours (their surface's own colour) follows that square.
 * These rooms carry no speaking rings: there is no audio for a call you are not in.
 */
async function checkFaces(target, scope) {
  const faces = await target.evaluate((scope) => {
    const avatars = [...document.querySelectorAll(`${scope} .live-stack .live-avatar`)].filter((element) =>
      element.querySelector("[data-avatar-asset] > .caper-picture"),
    );
    return avatars.map((element) => {
      const style = getComputedStyle(element);
      const surface = getComputedStyle(element.closest(".live-card, .voice-strip")).backgroundColor;
      return [
        element.querySelector("[data-avatar-asset]").dataset.avatarAsset,
        style.borderTopLeftRadius,
        style.overflow,
        style.boxShadow === `${surface} 0px 0px 0px 2px`,
        element.hasAttribute("data-speaking"),
      ];
    });
  }, scope);
  assert.deepEqual(
    faces.map(([asset]) => asset).sort(),
    ["gifpicture01", "stillpictur1"],
    `${scope}: animated and still picture faces`,
  );
  for (const [asset, ...face] of faces) assert.deepEqual(face, ["30%", "hidden", true, false], `${scope}: ${asset}`);
}

// Saving closes the modal or leaves /profile. Reopen through client-side history
// so the mock server and the application's saved account survive between steps.
async function reopenProfile(page, narrow = false) {
  if (narrow) {
    await page.waitForURL((url) => url.pathname === "/spaces");
    // The router changes the URL before committing the destination. Don't
    // interrupt that transition with Back while the old draft is still mounted.
    await page.getByRole("button", { name: "Change profile picture" }).waitFor({ state: "hidden" });
    await page.locator(".spaces-room").waitFor({ state: "attached" });
    await page.goBack();
    await page.waitForURL((url) => url.pathname === "/profile");
  } else {
    await page.locator(".profile-dialog").waitFor({ state: "hidden" });
    await page.locator(".account-profile").click();
  }
  await page.getByRole("button", { name: "Change profile picture" }).waitFor();
}

async function saveProfile(page, profile, narrow = false) {
  const before = await page.evaluate(() => window.pictureFixture.profileSaves.length);
  await profile.getByRole("button", { name: "Save profile" }).click();
  await reopenProfile(page, narrow);
  assert.equal(await page.evaluate(() => window.pictureFixture.profileSaves.length), before + 1);
  assert.ok(await profile.getByRole("button", { name: "Save profile" }).isDisabled(), "reopened saved form is clean");
}

async function checkProfileDraft(browser, narrow) {
  const label = narrow ? "narrow" : "desktop";
  const created = await context(browser, {
    viewport: { width: narrow ? 390 : 1280, height: narrow ? 844 : 900 },
    hasTouch: narrow,
    isMobile: narrow,
  });
  try {
    const page = await created.newPage();
    const problems = [];
    page.on("pageerror", (error) => problems.push(error.message));
    await page.goto(new URL(narrow ? "/profile" : "/spaces?space=space1234567&channel=channel12345", origin).href);
    if (!narrow) await page.locator(".account-profile").click();
    const profile = page.locator(narrow ? "main" : ".profile-dialog");
    const saved = await page.evaluate(() => window.pictureFixture.savedAccount());
    const discard = page.getByRole("dialog", { name: "Discard changes?", exact: true });
    const mascot = profile.locator(".picture-settings [data-avatar-id]");
    await mascot.waitFor();
    const beforeRegenerate = await page.evaluate(() => window.pictureFixture.requests.length);
    let previous = "0";
    for (let index = 0; index < 3; index++) {
      await profile.getByRole("button", { name: "Regenerate avatar" }).click();
      const next = await mascot.getAttribute("data-avatar-id");
      assert.notEqual(next, previous, "regenerate changes the local mascot");
      assert.ok(Number.isInteger(Number(next)) && Number(next) >= 0 && Number(next) < 800, "valid mascot ID");
      previous = next;
    }
    assert.deepEqual(
      await page.evaluate(
        (start) =>
          window.pictureFixture.requests
            .slice(start)
            .filter((request) => /^\w+ \/api\/(account|assets)(\/|$)/.test(request)),
        beforeRegenerate,
      ),
      [],
      "regenerate makes zero account/assets API calls (background space refreshes are independent)",
    );
    await profile.getByLabel("Username", { exact: true }).fill("draft_owner");
    await profile.getByLabel("Display name", { exact: true }).fill("Private draft");
    await profile.getByText("Unsaved changes", { exact: true }).waitFor();
    await profile.getByRole("button", { name: "Cancel", exact: true }).click();
    await discard.getByRole("button", { name: "Keep editing", exact: true }).click();
    assert.equal(await profile.getByLabel("Display name", { exact: true }).inputValue(), "Private draft");
    assert.equal(await mascot.getAttribute("data-avatar-id"), previous);
    if (narrow) await profile.getByRole("link", { name: "Back to spaces" }).click();
    else await profile.getByRole("button", { name: "Close profile" }).click();
    await discard.getByRole("button", { name: "Keep editing", exact: true }).click();
    if (narrow) await profile.getByRole("button", { name: "Cancel", exact: true }).click();
    else await page.keyboard.press("Escape");
    await discard.getByRole("button", { name: "Discard", exact: true }).click();
    await reopenProfile(page, narrow);
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.writes), []);
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.savedAccount()), saved);
    assert.equal(await mascot.getAttribute("data-avatar-id"), "0");
    assert.equal(await profile.getByLabel("Username", { exact: true }).inputValue(), saved.username);
    assert.equal(await profile.getByLabel("Display name", { exact: true }).inputValue(), saved.displayName);
    assert.ok(await profile.getByRole("button", { name: "Save profile" }).isDisabled());

    await profile
      .locator('.picture-settings input[type="file"][tabindex="-1"]')
      .setInputFiles({ name: "draft.png", mimeType: "image/png", buffer: landscape });
    const editor = page.getByRole("dialog", { name: "Adjust picture" });
    await editor.getByRole("button", { name: "Apply crop", exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.writes), [], "file choice does not upload");
    await editor.getByRole("button", { name: "Apply crop", exact: true }).click();
    await profile.getByText("Picture ready to save.").waitFor();
    const preview = profile.locator('.picture-settings .caper-picture img[src^="blob:"]');
    await preview.waitFor();
    await page.waitForFunction(() => document.querySelector('.picture-settings img[src^="blob:"]')?.naturalWidth > 0);
    const previewUrl = await preview.getAttribute("src");
    assert.ok(previewUrl.startsWith("blob:"));
    await profile.getByLabel("Username", { exact: true }).fill("saved_owner");
    await profile.getByLabel("Display name", { exact: true }).fill("Saved together");
    assert.deepEqual(
      await page.evaluate(() => window.pictureFixture.writes),
      [],
      "prepared picture and names stay local",
    );
    const assertPrivate = async () => {
      assert.deepEqual(await page.evaluate(() => window.pictureFixture.savedAccount()), saved);
      if (!narrow) {
        const visible = await page
          .locator(ownAvatars)
          .evaluateAll((elements) =>
            elements.map((element) => [element.dataset.avatarId, element.dataset.avatarAsset ?? null]),
          );
        assert.ok(visible.length >= 3, "check account, direct-self and chat avatars, not an empty selection");
        assert.ok(
          visible.every(([id, asset]) => id === "0" && asset === null),
          "private preview never replaces shared avatars",
        );
        assert.equal(await page.locator(".account-name").textContent(), saved.displayName);
      }
    };
    await assertPrivate();
    await page.evaluate(() => {
      window.pictureFixture.holdProfile = true;
      window.pictureFixture.failProfile = { status: 503, body: { error: "TEST FIXTURE: save unavailable" } };
    });
    await profile.getByRole("button", { name: "Save profile" }).click();
    await page.waitForFunction(() => typeof window.pictureFixture.finishProfile === "function");
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.writes), [
      "POST /api/assets",
      "POST /api/assets/upload000001/complete?draft=true",
      "POST /api/account/profile",
    ]);
    await assertPrivate(); // Completion must not publish before the atomic profile save.
    await page.evaluate(() => {
      window.pictureFixture.holdProfile = false;
      window.pictureFixture.finishProfile();
    });
    await profile.getByRole("alert").getByText("Your profile could not be saved.", { exact: false }).waitFor();
    await profile.getByText("Unsaved changes", { exact: true }).waitFor();
    assert.equal(await preview.getAttribute("src"), previewUrl, "failed save retains the prepared preview");
    assert.equal(await profile.getByLabel("Display name", { exact: true }).inputValue(), "Saved together");
    await assertPrivate();
    await shot(narrow ? page : profile, `profile-draft-failed-${label}.png`);
    await saveProfile(page, profile, narrow);
    const body = {
      username: "saved_owner",
      displayName: "Saved together",
      avatar: { avatarId: 0, avatarAsset: "upload000001", expectedAvatarId: 0, expectedAvatarAsset: null },
    };
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.profileSaves), [body, body]);
    assert.equal(
      await page.evaluate(() => window.pictureFixture.reservations.length),
      1,
      "retry reuses completed draft upload",
    );
    assert.equal(await page.evaluate(() => window.pictureFixture.completes), 1);
    assert.equal(
      await profile.locator(".picture-settings [data-avatar-asset]").getAttribute("data-avatar-asset"),
      "upload000001",
    );

    const beforeRemove = await page.evaluate(() => window.pictureFixture.writes.length);
    await profile.getByRole("button", { name: "Change profile picture" }).click();
    await profile.getByRole("menuitem", { name: "Remove picture" }).click();
    await profile.getByText("Removal ready to save.").waitFor();
    assert.equal(await page.evaluate(() => window.pictureFixture.writes.length), beforeRemove);
    assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), "upload000001");
    await profile.getByRole("button", { name: "Cancel", exact: true }).click();
    await discard.getByRole("button", { name: "Discard", exact: true }).click();
    await reopenProfile(page, narrow);
    assert.equal(
      await profile.locator(".picture-settings [data-avatar-asset]").getAttribute("data-avatar-asset"),
      "upload000001",
      "discard restores removed picture",
    );
    await profile.getByLabel("Display name", { exact: true }).fill("Names only");
    await saveProfile(page, profile, narrow);
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.profileSaves.at(-1)), {
      username: "saved_owner",
      displayName: "Names only",
    });
    const beforeMascotSave = await page.evaluate(() => window.pictureFixture.writes.length);
    await profile.getByRole("button", { name: "Change profile picture" }).click();
    await profile.getByRole("menuitem", { name: "Remove picture" }).click();
    await profile.getByRole("button", { name: "Regenerate avatar" }).click();
    const regenerated = Number(await mascot.getAttribute("data-avatar-id"));
    assert.notEqual(regenerated, 0);
    assert.equal(await page.evaluate(() => window.pictureFixture.writes.length), beforeMascotSave);
    await saveProfile(page, profile, narrow);
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.profileSaves.at(-1).avatar), {
      avatarId: regenerated,
      avatarAsset: null,
      expectedAvatarId: 0,
      expectedAvatarAsset: "upload000001",
    });
    assert.equal(
      Number(await mascot.getAttribute("data-avatar-id")),
      regenerated,
      "saved regeneration survives reopening",
    );
    assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), undefined);
    assert.equal(await page.evaluate(() => window.pictureFixture.reservations.length), 1);
    assert.deepEqual(problems, []);
    console.log(
      `PASS profile draft ${label}: local regeneration/crop/removal, zero writes, Cancel/Close/Escape/navigation guards, keep/discard, private failed-save preview, draft completion, atomic names+avatar, retry without reupload, names-only save.`,
    );
  } finally {
    await created.close();
  }
}

async function checkGallery(browser, narrow) {
  const label = narrow ? "narrow" : "desktop";
  const created = await context(browser, {
    viewport: { width: narrow ? 390 : 1280, height: narrow ? 844 : 900 },
    hasTouch: narrow,
    isMobile: narrow,
  });
  try {
    const page = await created.newPage();
    const problems = [];
    page.on("pageerror", (error) => problems.push(error.message));
    await page.goto(new URL(narrow ? "/profile" : "/spaces?space=space1234567&channel=channel12345", origin).href);
    if (narrow) assert.equal(await page.evaluate(() => matchMedia("(pointer: coarse)").matches), true);
    if (!narrow) await page.locator(".account-profile").click();
    const profile = page.locator(narrow ? "main" : ".profile-dialog");
    const edit = profile.getByRole("button", { name: "Change profile picture" });
    const open = async () => {
      await edit.click();
      await profile.getByRole("menuitem", { name: "Previous pictures" }).click();
    };
    const gallery = page.getByRole("dialog", { name: "Previous pictures", exact: true });
    await open();
    await gallery.getByText("No previous pictures yet.", { exact: false }).waitFor();
    assert.ok(await gallery.getByRole("button", { name: "Use picture", exact: true }).isDisabled());
    await gallery.evaluate((element) => Promise.all(element.getAnimations().map((animation) => animation.finished)));
    const bounds = await gallery.boundingBox();
    assert.ok(
      bounds && bounds.y >= 0 && bounds.y + bounds.height <= page.viewportSize().height,
      `Gallery must fit the viewport, including when opened from a scrolled profile: ${JSON.stringify(bounds)}`,
    );
    const header = await gallery.getByRole("heading", { name: "Previous pictures" }).boundingBox();
    assert.ok(header && header.y >= bounds.y, `Gallery title must stay visible: ${JSON.stringify({ bounds, header })}`);
    await shot(narrow ? page : gallery, `gallery-empty-${label}.png`);
    await gallery.getByRole("button", { name: "Cancel", exact: true }).click();
    assert.equal(await page.evaluate(() => window.pictureFixture.selections.length), 0);

    // Four retained, visibly distinct fixtures; page two contains the oldest choice.
    await page.evaluate(() => {
      window.pictureFixture.history = ["gifpicture01", "stillpictur1", "inviteicon01", "spaceicon001"];
      window.pictureFixture.pageSize = 2;
      window.pictureFixture.holdHistory = true;
    });
    await open();
    await gallery.getByText("Loading pictures…").waitFor();
    await shot(narrow ? page : gallery, `gallery-loading-${label}.png`);
    await page.evaluate(() => {
      window.pictureFixture.holdHistory = false;
      window.pictureFixture.finishHistory();
    });
    await gallery.getByRole("button", { name: "Load more", exact: true }).click();
    await gallery.getByRole("button", { name: "Previous picture 4", exact: true }).waitFor();
    assert.equal(await gallery.locator(".avatar-history-choice").count(), 4);
    await gallery.getByRole("button", { name: "Previous picture 3", exact: true }).click();
    assert.equal(
      await gallery.locator(".avatar-history-preview [data-avatar-asset]").getAttribute("data-avatar-asset"),
      "inviteicon01",
    );
    assert.equal(
      await profile.locator(".picture-settings [data-avatar-asset]").count(),
      0,
      "preview leaves the saved mascot alone",
    );
    assert.equal(await page.evaluate(() => window.pictureFixture.selections.length), 0);
    await shot(narrow ? page : gallery, `gallery-selected-${label}.png`);
    if (narrow) await page.goBack();
    else await page.keyboard.press("Escape");
    await gallery.waitFor({ state: "detached" });
    assert.ok(await profile.isVisible(), "dismiss only the gallery");
    assert.ok(await edit.evaluate((button) => document.activeElement === button));
    assert.equal(await page.evaluate(() => window.pictureFixture.selections.length), 0);
    await open();
    await gallery.getByRole("button", { name: "Load more", exact: true }).click();
    await gallery.getByRole("button", { name: "Previous picture 3", exact: true }).click();
    await gallery.getByRole("button", { name: "Use picture", exact: true }).click();
    await gallery.waitFor({ state: "detached" });
    assert.equal(
      await profile.locator(".picture-settings [data-avatar-asset]").getAttribute("data-avatar-asset"),
      "inviteicon01",
    );
    await profile.getByText("Unsaved changes", { exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.writes), [], "history choice is local");
    assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), undefined);
    assert.equal(await page.evaluate(() => window.pictureFixture.reservations.length), 0, "select never uploads");
    await open();
    await gallery.getByRole("button", { name: "Load more", exact: true }).click();
    await gallery.getByRole("button", { name: "Current picture 3", exact: true }).waitFor();
    assert.equal(
      await gallery.getByRole("button", { name: "Current picture 3", exact: true }).getAttribute("aria-pressed"),
      "true",
    );
    assert.ok(await gallery.getByRole("button", { name: "Use picture", exact: true }).isDisabled());
    await shot(narrow ? page : gallery, `gallery-current-${label}.png`);
    await gallery.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.evaluate(() => {
      window.pictureFixture.holdProfile = true;
    });
    await profile.getByRole("button", { name: "Save profile" }).click();
    await profile.getByRole("button", { name: "Saving…", exact: true }).waitFor();
    assert.ok(await edit.isDisabled(), "picture controls disabled while profile saves");
    assert.ok(await profile.getByLabel("Display name", { exact: true }).isDisabled());
    assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), undefined);
    await shot(narrow ? page : profile, `gallery-saving-${label}.png`);
    await page.evaluate(() => {
      window.pictureFixture.holdProfile = false;
      window.pictureFixture.finishProfile();
    });
    await reopenProfile(page, narrow);
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.profileSaves), [
      {
        username: "fixture_owner",
        displayName: "Owner (test)",
        avatar: { avatarId: 0, avatarAsset: "inviteicon01", expectedAvatarId: 0, expectedAvatarAsset: null },
      },
    ]);
    await open();
    await gallery.getByRole("button", { name: "Previous picture 1", exact: true }).click();
    await gallery.getByRole("button", { name: "Use picture", exact: true }).click();
    await gallery.waitFor({ state: "detached" });
    // A real stale baseline, not a special-case response: another device changed it.
    await page.evaluate(() => window.pictureFixture.updateAccount({ avatarId: 42 }));
    await profile.getByRole("button", { name: "Save profile" }).click();
    await profile.getByRole("alert").getByText("Your picture changed elsewhere.", { exact: false }).waitFor();
    assert.deepEqual(await page.evaluate(() => window.pictureFixture.profileSaves.at(-1).avatar), {
      avatarId: 0,
      avatarAsset: "gifpicture01",
      expectedAvatarId: 0,
      expectedAvatarAsset: "inviteicon01",
    });
    assert.equal(
      await profile.locator(".picture-settings [data-avatar-asset]").getAttribute("data-avatar-asset"),
      "gifpicture01",
    );
    assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), "inviteicon01");
    await profile.getByText("Unsaved changes", { exact: true }).waitFor();
    await shot(narrow ? page : profile, `gallery-conflict-${label}.png`);
    await page.evaluate(() => {
      window.pictureFixture.failHistory = true;
    });
    await open();
    await gallery.getByRole("button", { name: "Retry", exact: true }).waitFor();
    await shot(narrow ? page : gallery, `gallery-error-${label}.png`);
    await page.evaluate(() => {
      window.pictureFixture.failHistory = false;
    });
    await gallery.getByRole("button", { name: "Retry", exact: true }).click();
    await gallery.getByRole("button", { name: "Current picture 1", exact: true }).waitFor();
    await gallery.getByRole("button", { name: "Cancel", exact: true }).click();
    assert.deepEqual(problems, []);
    console.log(
      `PASS gallery ${label}: preview-only, pagination, Cancel/Escape/Back, local selection, zero writes until Save profile, current draft, pending, stale conflict retains draft, empty/error/retry.`,
    );
  } finally {
    await created.close();
  }
}

async function checkExistingPictures(browser) {
  // ---- Desktop: rendering everywhere ----
  const desktop = await context(browser, { viewport: { width: 1280, height: 900 } });
  const page = await desktop.newPage();
  const problems = [];
  page.on("pageerror", (error) => problems.push(error.message));
  await page.goto(new URL("/spaces?space=space1234567&channel=channel12345", origin).href);
  await page.waitForFunction(loaded('.chat-avatar [data-avatar-asset="gifpicture01"]'));
  await page.waitForFunction(loaded('.chat-avatar [data-avatar-asset="stillpictur1"]'));
  // A failed picture is removed, leaving the saved mascot (never a broken image).
  await page.waitForFunction(
    '!document.querySelector(\'.chat-avatar [data-avatar-asset="brokenpictr1"] .caper-picture\') && document.querySelector(\'.chat-avatar [data-avatar-asset="brokenpictr1"]\')?.dataset.avatarId === "799"',
  );
  assert.equal(await page.locator('.chat-avatar [data-avatar-id="143"] .caper-picture').count(), 0);
  await page.waitForFunction(loaded(".space-rail-item:first-child button"));
  await page.waitForFunction(loaded(".space-rail .invited-space"));
  assert.equal(await page.locator(".space-rail-item:nth-child(2) button .caper-picture").count(), 0);
  assert.equal(await page.locator(".space-rail-item:nth-child(2) button > span").first().textContent(), "G");
  // Badges stay on top of icons: the picture comes before them in paint order.
  assert.equal(
    await page.evaluate(
      () => getComputedStyle(document.querySelector(".space-rail-item:first-child .caper-picture")).position,
    ),
    "absolute",
  );
  // GIFs animate: two captures a third of a second apart differ.
  const maya = page.locator('.chat-avatar [data-avatar-asset="gifpicture01"]').first();
  const first = await maya.screenshot();
  await page.waitForTimeout(330);
  assert.notDeepEqual(await maya.screenshot(), first, "the animated GIF avatar should move");
  // The first click on a freshly loaded room can land before its handlers settle; try until it opens.
  for (let attempt = 0; attempt < 5 && !(await page.locator("#space-member-list").count()); attempt++) {
    await page.locator(".member-list-toggle").click();
    await page.waitForTimeout(300);
  }
  await page.waitForFunction(loaded('.member-presence-avatar [data-avatar-asset="gifpicture01"]'));
  await page.waitForFunction(
    loaded(
      '.live-room .live-avatar [data-avatar-asset="stillpictur1"], .live-room .live-avatar [data-avatar-asset="gifpicture01"]',
    ),
  );
  await shot(page, "pictures-desktop.png");
  // Profile cards show the picture large.
  await page.locator('.chat-avatar[aria-label="Open profile for Maya (animated GIF)"]').click();
  await page.waitForFunction(loaded('.chat-mention-card [data-avatar-asset="gifpicture01"]'));
  await page.waitForTimeout(300);
  await shot(page.locator(".chat-mention-card"), "pictures-profile-card.png");
  await page.keyboard.press("Escape");
  await page.waitForFunction('!document.querySelector(".chat-mention-card")');
  await shot(page.locator(".space-rail"), "pictures-rail-desktop.png");
  await checkSquares(page, "desktop");

  // The room's faces on its live card keep the pictures' rounded squares.
  await checkFaces(page, ".live-room");
  await shot(page.locator(".live-room"), "pictures-live-room-desktop.png");

  // ---- Profile picture: upload a still (cropped, WebP), then a GIF (unchanged), then remove ----
  await page.locator(".account-profile").click();
  const dialog = page.locator(".profile-dialog");
  // Even without a custom picture, the menu offers the retained picture library.
  const editPicture = dialog.getByRole("button", { name: "Change profile picture" });
  await editPicture.waitFor();
  await dialog.locator('.picture-settings-change-me[data-avatar-id="0"]').waitFor();
  assert.equal(await editPicture.getAttribute("aria-haspopup"), "menu");
  await page.waitForTimeout(300); // Let the sign's vector arrive.
  await shot(dialog, "pictures-profile-mascot.png");
  await dialog.locator('.picture-settings input[type="file"][tabindex="-1"]').setInputFiles({
    name: "landscape-photo.png",
    mimeType: "image/png",
    buffer: landscape,
  });
  const editor = page.getByRole("dialog", { name: "Adjust picture" });
  await editor.getByRole("button", { name: "Apply crop", exact: true }).waitFor();
  await page.waitForFunction(() => !!document.querySelector(".chat-editor-crop"));
  assert.equal(await page.evaluate(() => window.pictureFixture.reservations.length), 0, "choosing does not upload");
  const cropFrame = editor.getByRole("group", { name: "Crop frame" });
  await cropFrame.focus();
  for (let i = 0; i < 20; i++) await page.keyboard.press("ArrowLeft");
  await shot(page, "pictures-crop-desktop.png");
  await editor.getByRole("button", { name: "Apply crop", exact: true }).click();
  await dialog.getByText("Picture ready to save.").waitFor();
  assert.deepEqual(await page.evaluate(() => window.pictureFixture.writes), [], "Apply crop is local");
  await saveProfile(page, dialog);
  const still = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.equal(still.purpose, "avatar");
  assert.equal(still.contentType, "image/webp");
  assert.equal(still.filename, "avatar.webp");
  assert.equal(still.width, 512);
  assert.equal(still.height, 512);
  assert.ok(still.byteSize > 0 && still.byteSize <= 1024 * 1024);
  const sampled = await page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
    const image = await createImageBitmap(new Blob([bytes], { type: "image/webp" }));
    const canvas = new OffscreenCanvas(image.width, image.height);
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    return [context.getImageData(4, 256, 1, 1).data[0], context.getImageData(507, 256, 1, 1).data[0]];
  }, stored.get("upload000001").body.toString("base64"));
  assert.ok(
    sampled[0] < 10 && sampled[1] > 180 && sampled[1] < 200,
    `saved crop must contain the left 600px, not the centered 600px: ${sampled}`,
  );
  // Your own picture updates everywhere without a reload.
  await page.waitForFunction(
    `[...document.querySelectorAll(${JSON.stringify(ownAvatars)})].length >= 3 && [...document.querySelectorAll(${JSON.stringify(ownAvatars)})].every((element) => element.dataset.avatarAsset === "upload000001" && element.querySelector(".caper-picture")?.dataset.state !== "loading")`,
  );
  await page.waitForFunction(loaded('.member-presence-avatar [data-avatar-asset="upload000001"]'));
  // With one, the frame offers a new picture or removal; the sign steps aside.
  assert.equal(await editPicture.getAttribute("aria-haspopup"), "menu");
  assert.equal(await dialog.locator(".picture-settings-change-me").count(), 0);
  await editPicture.click();
  await dialog.getByRole("menuitem", { name: "Remove picture" }).waitFor();
  assert.equal(await dialog.getByRole("menuitem", { name: "Previous pictures" }).count(), 1);
  await shot(dialog, "pictures-profile-uploaded.png");
  await dialog.getByRole("menuitem", { name: "Edit picture", exact: true }).click();
  await editor.getByRole("slider", { name: "Crop size" }).waitFor();
  await editor.getByRole("slider", { name: "Crop size" }).focus();
  await page.keyboard.press("Home");
  await editor.getByRole("button", { name: "Rotate left" }).click();
  await editor.getByRole("button", { name: "Cancel editing" }).click();
  assert.equal(
    await page.evaluate(() => window.pictureFixture.reservations.length),
    1,
    "cancel current-picture editing never uploads",
  );
  await editPicture.click();
  await page.keyboard.press("Escape");
  await dialog.getByRole("menu").waitFor({ state: "detached" });
  assert.ok(await dialog.isVisible(), "Escape closes the menu, not the dialog");
  await dialog.locator('.picture-settings input[type="file"][tabindex="-1"]').setInputFiles({
    name: "party.gif",
    mimeType: "image/gif",
    buffer: animatedAvatar,
  });
  await shot(dialog, "pictures-animation-choice.png");
  await dialog.getByRole("button", { name: "Keep original animation" }).click();
  await saveProfile(page, dialog);
  await page.waitForFunction(() => window.pictureFixture.completes === 2);
  const animated = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.deepEqual(
    [animated.contentType, animated.filename, animated.byteSize, animated.width, animated.height],
    ["image/gif", "avatar.gif", animatedAvatar.length, 64, 64],
  );
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000002"]'));
  // The library includes completed uploads; selecting one drafts it without another upload.
  await editPicture.click();
  const previousPictures = dialog.getByRole("menuitem", { name: "Previous pictures" });
  await previousPictures.click();
  const gallery = page.getByRole("dialog", { name: "Previous pictures", exact: true });
  await gallery.getByRole("button", { name: "Previous picture 2", exact: true }).click();
  await gallery.getByRole("button", { name: "Use picture", exact: true }).click();
  await gallery.waitFor({ state: "detached" });
  assert.ok(await editPicture.evaluate((button) => document.activeElement === button));
  await saveProfile(page, dialog);
  await page.waitForFunction(
    `[...document.querySelectorAll(${JSON.stringify(ownAvatars)})].every((element) => element.dataset.avatarAsset === "upload000001")`,
  );
  await editPicture.click();
  await previousPictures.click();
  await gallery.getByRole("button", { name: "Previous picture 1", exact: true }).click();
  await gallery.getByRole("button", { name: "Use picture", exact: true }).click();
  await gallery.waitFor({ state: "detached" });
  await saveProfile(page, dialog);
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000002"]'));
  assert.equal(await page.evaluate(() => window.pictureFixture.reservations.length), 2);
  // Local and server errors are explained inline.
  await dialog.locator('.picture-settings input[type="file"][tabindex="-1"]').setInputFiles({
    name: "huge.gif",
    mimeType: "image/gif",
    buffer: oversizedGif,
  });
  assert.ok(oversizedGif.length > 5 * 1024 * 1024);
  await dialog.getByRole("button", { name: "Keep original animation" }).click();
  await dialog.getByText("Picture ready to save.").waitFor({ timeout: 60_000 });
  await saveProfile(page, dialog);
  await page.waitForFunction(() => window.pictureFixture.completes === 3, undefined, { timeout: 60_000 });
  const shrunk = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.deepEqual(
    [shrunk.contentType, shrunk.filename, shrunk.width, shrunk.height],
    ["image/gif", "avatar.gif", 512, 512],
  );
  assert.ok(shrunk.byteSize <= 5 * 1024 * 1024);
  // Chrome's own decoder plays the shrunk upload: every frame, square.
  const shrunkBody = stored.get("upload000003").body;
  assert.equal(shrunkBody.length, shrunk.byteSize);
  const playback = await page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
    const decoder = new ImageDecoder({ data: bytes, type: "image/gif" });
    await decoder.tracks.ready;
    const { image } = await decoder.decode({ frameIndex: 0 });
    return {
      frames: decoder.tracks.selectedTrack.frameCount,
      width: image.displayWidth,
      height: image.displayHeight,
    };
  }, shrunkBody.toString("base64"));
  assert.deepEqual(playback, { frames: 12, width: 512, height: 512 });
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000003"]'));
  await dialog.locator('.picture-settings input[type="file"][tabindex="-1"]').setInputFiles({
    name: "photo.heic",
    mimeType: "image/heic",
    buffer: Buffer.from("not a picture this browser can decode"),
  });
  await editor.getByText("This file can’t be edited here.").waitFor();
  await editor.getByRole("button", { name: "Cancel editing" }).click();
  await page.evaluate(() => {
    window.pictureFixture.failNext = { status: 429, body: { error: "too many", code: "picture_rate_limited" } };
  });
  await dialog.locator('.picture-settings input[type="file"][tabindex="-1"]').setInputFiles({
    name: "again.gif",
    mimeType: "image/gif",
    buffer: animatedAvatar,
  });
  await dialog.getByRole("button", { name: "Keep original animation" }).click();
  await dialog.getByRole("button", { name: "Save profile" }).click();
  await dialog.getByText("You’ve changed pictures too many times today. Try again tomorrow.").waitFor();
  await shot(dialog, "pictures-profile-error.png");
  await editPicture.click();
  await dialog.getByRole("menuitem", { name: "Remove picture" }).click();
  await dialog.getByText("Removal ready to save.").waitFor();
  assert.equal(await page.evaluate(() => window.pictureFixture.removals), 0, "remove does not call legacy API");
  assert.equal(await page.evaluate(() => window.pictureFixture.savedAccount().avatarAsset), "upload000003");
  await saveProfile(page, dialog);
  await page.waitForFunction(
    `[...document.querySelectorAll(${JSON.stringify(ownAvatars)})].every((element) => !element.dataset.avatarAsset && !element.querySelector(".caper-picture") && element.dataset.avatarId === "0")`,
  );
  await dialog.locator(".picture-settings-change-me").waitFor();
  assert.equal(await editPicture.getAttribute("aria-haspopup"), "menu");
  await editPicture.click();
  assert.equal(await dialog.getByRole("menuitem", { name: "Remove picture" }).count(), 0);
  assert.equal(await dialog.getByRole("menuitem", { name: "Edit picture", exact: true }).count(), 0);
  await previousPictures.click();
  await gallery.getByRole("button", { name: "Previous picture 1", exact: true }).click();
  await gallery.getByRole("button", { name: "Use picture", exact: true }).click();
  await gallery.waitFor({ state: "detached" });
  await saveProfile(page, dialog);
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000003"]'));
  await dialog.getByRole("button", { name: "Close profile" }).click();

  // ---- Space icon: remove, then upload a new one; the rail follows ----
  await page.locator(".space-menu summary").click();
  await page.getByRole("button", { name: "Space settings" }).click();
  const manage = page.locator(".space-dialog");
  await manage.locator(".space-icon-section").getByRole("button", { name: "Change space icon" }).waitFor();
  await page.waitForFunction(loaded(".space-icon-section .picture-settings-tile"));
  await shot(manage.locator(".space-icon-section"), "pictures-space-icon-settings.png");
  await manage.getByRole("button", { name: "Change space icon" }).click();
  await manage.getByRole("menuitem", { name: "Remove icon" }).click();
  await manage.getByText("Space icon removed.").waitFor();
  await page.waitForFunction(
    '!document.querySelector(".space-rail-item:first-child .caper-picture") && document.querySelector(".space-rail-item:first-child button > span").textContent === "T"',
  );
  const iconButton = manage.locator(".picture-settings-edit");
  const drop = await page.evaluateHandle(
    (bytes) => {
      const data = new DataTransfer();
      data.items.add(new File([new Uint8Array(bytes)], "icon.png", { type: "image/png" }));
      // A picture editor uses the first file, not the last or every file.
      data.items.add(new File(["not a picture"], "notes.txt", { type: "text/plain" }));
      return data;
    },
    [...stillIcon],
  );
  await iconButton.dispatchEvent("dragover", { dataTransfer: drop });
  await shot(manage.locator(".space-icon-section"), "pictures-space-drop-target.png");
  await iconButton.dispatchEvent("drop", { dataTransfer: drop });
  await editor.getByRole("button", { name: "Done", exact: true }).click();
  await manage.getByText("Space icon updated.").waitFor();
  const iconReservation = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.deepEqual(
    [iconReservation.purpose, iconReservation.spaceId, iconReservation.filename, iconReservation.width],
    ["spaceIcon", "space1234567", "space-icon.webp", 64],
  );
  await page.waitForFunction(loaded(".space-rail-item:first-child button"));
  await shot(manage, "pictures-space-dialog.png");
  // Replacement bypasses the menu, and invalid files keep the existing picture.
  let releaseUpload;
  const uploadHeld = new Promise((resolve) => (releaseUpload = resolve));
  await page.route("https://r2.test/put/**", async (route) => {
    await uploadHeld;
    await route.fallback();
  });
  const beforeReplacement = await page.evaluate(() => window.pictureFixture.reservations.length);
  await iconButton.click();
  await iconButton.dispatchEvent("drop", { dataTransfer: drop });
  await manage.locator('.picture-settings[aria-busy="true"]').waitFor();
  await iconButton.dispatchEvent("drop", { dataTransfer: drop });
  await editor.getByRole("button", { name: "Done", exact: true }).click();
  releaseUpload();
  await manage.getByText("Space icon updated.").waitFor();
  await page.unroute("https://r2.test/put/**");
  assert.equal(
    await page.evaluate(() => window.pictureFixture.reservations.length),
    beforeReplacement + 1,
    "a second drop during preparation/upload must not start another upload",
  );
  assert.equal(await manage.getByRole("menu").count(), 0);
  const reservations = await page.evaluate(() => window.pictureFixture.reservations.length);
  const invalid = await page.evaluateHandle(() => {
    const data = new DataTransfer();
    data.items.add(new File(["not a picture"], "notes.txt", { type: "text/plain" }));
    return data;
  });
  await iconButton.dispatchEvent("drop", { dataTransfer: invalid });
  await editor.getByText("This file can’t be edited here.").waitFor();
  await editor.getByRole("button", { name: "Cancel editing" }).click();
  assert.equal(await page.evaluate(() => window.pictureFixture.reservations.length), reservations);
  await page.waitForFunction(loaded(".space-rail-item:first-child button"));
  await drop.dispose();
  await invalid.dispose();
  await manage.getByRole("button", { name: "Close Manage space" }).click();
  await shot(page.locator(".space-rail"), "pictures-rail-updated.png");

  // ---- Live updates: another person, yourself (another device) and the space change elsewhere ----
  await page.waitForFunction(() => typeof window.pictureFixture.pushLive === "function");
  const profilesFrame = await page.evaluate(() => window.pictureFixture.profilesFrame);
  assert.deepEqual(Object.keys(profilesFrame).sort(), ["id", "kind", "type"], "subscribe with only a kind");
  if (!(await page.locator("#space-member-list").count())) await page.locator(".member-list-toggle").click();
  await page.evaluate(() => {
    const live = (asset) => `https://cdn.test/assets/${asset}?exp=1792086400&sig=live`;
    const push = window.pictureFixture.pushLive;
    // Maya removes her picture and renames; Avery adds one.
    push({
      type: "profile.updated",
      person: { id: "member000001", username: "maya_live", displayName: "Maya (renamed live)", avatarId: 31 },
    });
    push({
      type: "profile.updated",
      person: {
        id: "member000004",
        username: "avery",
        displayName: "Avery (picture live)",
        avatarId: 143,
        avatarAsset: "stillpictur1",
        avatarUrl: live("stillpictur1"),
      },
    });
    push({
      type: "space.updated",
      space: {
        id: "space1234567",
        name: "Studio (renamed live)",
        ownerId: "owner1234567",
        iconAsset: "inviteicon01",
        iconUrl: live("inviteicon01"),
      },
    });
    push({ type: "space.updated", space: { id: "notmine12345", name: "Not a space you have", ownerId: "x" } });
    // Your own account, changed on another device.
    const self = {
      username: "fixture_owner",
      displayName: "Owner (renamed live)",
      avatarId: 0,
      avatarAsset: "gifpicture01",
      avatarUrl: live("gifpicture01"),
    };
    window.pictureFixture.updateAccount(self);
    push({ type: "profile.updated", person: { id: "owner1234567", ...self } });
  });
  const names = (selector) =>
    page.evaluate((query) => [...document.querySelectorAll(query)].map((element) => element.textContent), selector);
  await page.waitForFunction(
    '[...document.querySelectorAll(".chat-author-name")].some((name) => name.textContent === "Maya (renamed live)") && [...document.querySelectorAll(".chat-author-name")].some((name) => name.textContent === "Owner (renamed live)")',
  );
  assert.ok((await names(".chat-author-name")).includes("Avery (picture live)"));
  assert.ok((await names(".chat-reply-heading strong")).includes("Maya (renamed live)"), "nested reply author");
  assert.ok((await names(".space-member-presence strong")).includes("Maya (renamed live)"), "member list");
  await page.waitForFunction(
    "!document.querySelector('.chat-avatar[aria-label=\"Open profile for Maya (renamed live)\"] .caper-picture')",
  );
  await page.waitForFunction(loaded('.chat-avatar[aria-label="Open profile for Avery (picture live)"]'));
  await page.waitForFunction(loaded(".account-avatar"));
  assert.equal(await page.locator(".account-name").textContent(), "Owner (renamed live)");
  await page.waitForFunction(
    'document.querySelector(".space-menu h1")?.textContent === "Studio (renamed live)" && !!document.querySelector(".space-rail-item:first-child .caper-picture img[src*=inviteicon01]")',
  );
  assert.ok(
    (await page.locator(".space-rail-item:first-child button").getAttribute("aria-label")).startsWith(
      "Studio (renamed live)",
    ),
  );
  assert.equal(await page.locator(".space-rail-item").count(), 2, "an update for a space you don't have is ignored");
  await page.waitForTimeout(300); // Let pictures finish arriving.
  await shot(page, "pictures-live-desktop.png");
  await checkSquares(page, "desktop after live updates");
  assert.deepEqual(problems, []);
  await desktop.close();

  // ---- Narrow Browse: the rail shows icons at phone width ----
  const narrow = await context(browser, { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const phone = await narrow.newPage();
  await phone.goto(new URL("/spaces?space=space1234567&channel=channel12345", origin).href);
  await phone.waitForFunction(loaded('.chat-avatar [data-avatar-asset="gifpicture01"]'));
  // Phones show the open channel's room in a strip under the conversation header.
  await phone.waitForFunction(
    loaded(
      '.voice-strip .live-avatar [data-avatar-asset="stillpictur1"], .voice-strip .live-avatar [data-avatar-asset="gifpicture01"]',
    ),
  );
  await checkFaces(phone, ".voice-strip");
  await shot(phone, "pictures-narrow-conversation.png");
  await phone.getByRole("button", { name: "Back to Browse" }).first().click();
  await phone.waitForFunction('!!document.querySelector(".spaces-room.navigation-open")');
  await phone.waitForFunction(loaded(".space-rail-item:first-child button"));
  await phone.waitForTimeout(400);
  await shot(phone, "pictures-narrow-browse.png");
  await checkSquares(phone, "narrow");
  // Browse shows the same room as a live card in its row.
  await phone.waitForFunction(loaded('.live-room .live-avatar [data-avatar-asset="gifpicture01"]'));
  await checkFaces(phone, ".live-room");
  await narrow.close();

  // ---- Phone-width profile page and invite page ----
  const pages = await context(browser, { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const profile = await pages.newPage();
  await profile.goto(new URL("/profile", origin).href);
  await profile.getByRole("button", { name: "Change profile picture" }).waitFor();
  await shot(profile, "pictures-profile-page-narrow.png");
  assert.equal(await profile.evaluate(() => matchMedia("(pointer: coarse)").matches), true);
  await profile
    .locator('.picture-settings input[type="file"][tabindex="-1"]')
    .setInputFiles({ name: "crop.png", mimeType: "image/png", buffer: landscape });
  const narrowEditor = profile.getByRole("dialog", { name: "Adjust picture" });
  const size = narrowEditor.getByRole("slider", { name: "Crop size" });
  await size.focus();
  await profile.keyboard.press("Home");
  for (let i = 0; i < 40; i++) await profile.keyboard.press("ArrowRight");
  const frame = await narrowEditor.getByRole("group", { name: "Crop frame" }).boundingBox();
  assert.ok(Math.abs(frame.width - frame.height) < 1, "zoom keeps a square frame");
  await narrowEditor.getByRole("button", { name: "Rotate left" }).tap();
  await shot(profile, "pictures-crop-narrow.png");
  await narrowEditor.getByRole("button", { name: "Apply crop", exact: true }).tap();
  await profile.getByText("Picture ready to save.").waitFor();
  assert.deepEqual(await profile.evaluate(() => window.pictureFixture.writes), []);
  await saveProfile(profile, profile.locator("main"), true);
  assert.deepEqual(
    await profile.evaluate(() => {
      const saved = window.pictureFixture.reservations.at(-1);
      return [saved.width, saved.height];
    }),
    [300, 300],
  );
  // A GIF can be explicitly edited as a still rather than silently flattened.
  await profile
    .locator('.picture-settings input[type="file"][tabindex="-1"]')
    .setInputFiles({ name: "still.gif", mimeType: "image/gif", buffer: animatedAvatar });
  await profile.getByRole("button", { name: "Edit still frame" }).tap();
  await narrowEditor.getByRole("button", { name: "Apply crop", exact: true }).tap();
  await saveProfile(profile, profile.locator("main"), true);
  await profile.waitForFunction(() => window.pictureFixture.completes === 2);
  assert.equal(await profile.evaluate(() => window.pictureFixture.reservations.at(-1).contentType), "image/webp");
  await profile.getByRole("button", { name: "Change profile picture" }).tap();
  await profile.getByRole("menuitem", { name: "Edit picture", exact: true }).tap();
  await narrowEditor.getByRole("button", { name: "Apply crop", exact: true }).tap();
  await saveProfile(profile, profile.locator("main"), true);
  await profile.waitForFunction(() => window.pictureFixture.completes === 3);
  assert.equal(
    await profile.evaluate(() => window.pictureFixture.reservations.at(-1).width),
    64,
    "current saved picture can be confirmed without choosing another file",
  );
  await profile.getByRole("button", { name: "Change profile picture" }).tap();
  await profile.getByRole("menuitem", { name: "Previous pictures" }).tap();
  const narrowGallery = profile.getByRole("dialog", { name: "Previous pictures", exact: true });
  await narrowGallery.getByRole("button", { name: "Previous picture 3", exact: true }).tap();
  await narrowGallery.getByRole("button", { name: "Use picture", exact: true }).tap();
  await narrowGallery.waitFor({ state: "detached" });
  assert.equal(
    await profile.locator(".picture-settings [data-avatar-asset]").getAttribute("data-avatar-asset"),
    "upload000001",
  );
  await saveProfile(profile, profile.locator("main"), true);
  await profile.goto(new URL("/invite/FIXTURE123", origin).href);
  await profile.waitForFunction(loaded("main [data-avatar-asset]"));
  await profile.waitForFunction(
    '!!document.querySelector("main .caper-picture img[src*=inviteicon01]") && [...document.querySelectorAll("main .caper-picture")].every((picture) => picture.dataset.state !== "loading")',
  );
  await profile.waitForTimeout(300); // Let pictures finish arriving.
  await shot(profile, "pictures-invite-narrow.png");
  await pages.close();

  // ---- Reduced motion: a still frame instead of animation ----
  const calm = await context(browser, { viewport: { width: 1280, height: 900 }, reducedMotion: "reduce" });
  const quiet = await calm.newPage();
  await quiet.goto(new URL("/spaces?space=space1234567&channel=channel12345", origin).href);
  await quiet.waitForFunction(loaded('.chat-avatar [data-avatar-asset="gifpicture01"]'));
  await quiet.waitForFunction(
    '[...document.querySelectorAll(".caper-picture")].filter((picture) => picture.dataset.state !== "loading").every((picture) => picture.hasAttribute("data-still") && picture.querySelector("canvas")?.width <= 256 && getComputedStyle(picture.querySelector("img")).opacity === "0")',
  );
  const calmMaya = quiet.locator('.chat-avatar [data-avatar-asset="gifpicture01"]').first();
  const calmFirst = await calmMaya.screenshot();
  await quiet.waitForTimeout(330);
  assert.deepEqual(await calmMaya.screenshot(), calmFirst, "reduced motion shows a still frame");
  const calmIcon = quiet.locator(".space-rail-item:first-child button");
  const iconFirst = await calmIcon.screenshot();
  await quiet.waitForTimeout(330);
  assert.deepEqual(await calmIcon.screenshot(), iconFirst, "reduced motion keeps space icons still");
  await shot(quiet, "pictures-reduced-motion.png");
  await calm.close();
  console.log(
    "PASS: animated, still, broken and absent pictures; rounded squares, including live room card and phone voice strip faces; upload, GIF, errors and removal; space icons; live profile and space updates; narrow; reduced motion.",
  );
}

async function checkProfileViewer(browser, narrow, reduced = false) {
  const label = reduced ? "reduced-motion" : narrow ? "touch" : "desktop";
  const created = await context(browser, {
    viewport: narrow ? { width: 390, height: 844 } : { width: 1280, height: 900 },
    hasTouch: narrow,
    isMobile: narrow,
    reducedMotion: reduced ? "reduce" : "no-preference",
    permissions: ["clipboard-read", "clipboard-write"],
  });
  // An asymmetric original proves the viewer shows the original, not the square thumbnail.
  await created.route("https://cdn.test/assets/stillpictur1*", (route) =>
    route.fulfill({ body: landscape, contentType: "image/png", headers: { "access-control-allow-origin": "*" } }),
  );
  const page = await created.newPage();
  const problems = [];
  page.on("pageerror", (error) => problems.push(error.message));
  try {
    await page.goto(new URL("/spaces?space=space1234567&channel=channel12345", origin).href);
    if (narrow) assert.equal(await page.evaluate(() => matchMedia("(pointer: coarse)").matches), true);
    const who = reduced ? "Maya (animated GIF)" : "June (still)";
    const avatar = page.locator(`.chat-avatar[aria-label="Open profile for ${who}"]`);
    await avatar.click();
    const card = page.locator(".chat-mention-card");
    const picture = card.getByRole("button", { name: `Open ${who}’s profile picture`, exact: true });
    await picture.locator("img").waitFor();
    await card.locator("textarea").fill("TEST FIXTURE — keep this draft");
    // Browser-native image actions must hit the real image and remain uncancelled.
    const nativeMenu = await picture.locator("img").evaluate((image) => {
      const box = image.getBoundingClientRect();
      const target = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      return { image: target === image, allowed: target.dispatchEvent(event) };
    });
    assert.deepEqual(nativeMenu, { image: true, allowed: true });
    await picture.click();
    const viewer = page.getByRole("dialog", { name: "Image viewer", exact: true });
    await viewer.waitFor();
    await page.waitForFunction(() => document.querySelector(".chat-viewer-image img")?.naturalWidth > 0);
    const image = viewer.locator(".chat-viewer-image");
    if (reduced) {
      const first = await image.screenshot();
      await page.waitForTimeout(330);
      assert.deepEqual(await image.screenshot(), first, "full-size GIF stays still with reduced motion");
    } else {
      const bounds = await image.boundingBox();
      assert.ok(bounds.width > 250 && Math.abs(bounds.width / bounds.height - 4 / 3) < 0.01);
    }
    await shot(page, `profile-picture-${label}.png`);
    await viewer.getByLabel("More media actions", { exact: true }).click();
    if (!narrow) {
      await viewer.getByRole("button", { name: "Copy image", exact: true }).click();
      await viewer.getByText("Image copied", { exact: true }).waitFor();
      assert.deepEqual(
        await page.evaluate(async () => {
          const item = (await navigator.clipboard.read())[0];
          const bitmap = await createImageBitmap(await item.getType("image/png"));
          const dimensions = [bitmap.width, bitmap.height];
          bitmap.close();
          return dimensions;
        }),
        reduced ? [64, 64] : [800, 600],
        "clipboard contains the original pixels, not an avatar-sized thumbnail",
      );
    }
    const browserLink = viewer.getByRole("link", { name: "Open in browser", exact: true });
    const [opened] = await Promise.all([created.waitForEvent("page"), browserLink.click()]);
    await opened.waitForLoadState();
    assert.ok(opened.url().includes(reduced ? "gifpicture01" : "stillpictur1"));
    await opened.close();
    if (narrow) await page.goBack();
    else await page.keyboard.press("Escape");
    await viewer.waitFor({ state: "detached" });
    assert.ok(await card.isVisible(), "closing only the viewer preserves the profile");
    assert.equal(await card.locator("textarea").inputValue(), "TEST FIXTURE — keep this draft");
    await picture.focus();
    await page.keyboard.press("Enter");
    await viewer.waitFor();
    await viewer.getByRole("button", { name: "Close viewer" }).click();
    await viewer.waitFor({ state: "detached" });
    await card.getByRole("button", { name: "Close profile" }).click();
    await page.locator('.chat-avatar[aria-label="Open profile for Avery (mascot)"]').click();
    assert.equal(
      await card.locator("button.chat-mention-card-avatar").count(),
      0,
      "mascots don't open a missing picture",
    );
    assert.deepEqual(problems, []);
    console.log(
      `PASS profile picture ${label}: original, native context target, copy/open, close/Back, keyboard, draft, mascot.`,
    );
  } finally {
    await created.close();
  }
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH });
try {
  if (process.env.PICTURE_VIEWER_ONLY !== "1") {
    await checkProfileDraft(browser, false);
    await checkProfileDraft(browser, true);
  }
  await checkProfileViewer(browser, false);
  await checkProfileViewer(browser, true);
  await checkProfileViewer(browser, false, true);
  if (process.env.PICTURE_VIEWER_ONLY !== "1") {
    await checkGallery(browser, false);
    await checkGallery(browser, true);
    if (process.env.PICTURE_GALLERY_ONLY !== "1") await checkExistingPictures(browser);
  }
} finally {
  await browser.close();
}
