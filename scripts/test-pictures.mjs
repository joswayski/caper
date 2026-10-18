// Custom profile pictures and space icons in the real web UI, against explicitly
// labelled in-page API/gateway mocks and a fake CDN/storage served by Playwright.
// No real accounts, uploads, CDN or SFU calls.
//
//   PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/index.mjs \
//   PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers \
//   node scripts/test-pictures.mjs http://localhost:5174 [screenshot-directory]
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
  const account = { id: "owner1234567", username: "fixture_owner", displayName: "Owner (test)", avatarId: 0 };
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
  const control = (window.pictureFixture = { reservations: [], completes: 0, removals: 0, failNext: undefined });
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
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    const method = (options.method ?? "GET").toUpperCase();
    if (!path.startsWith("/api/")) return original(input, options);
    if (path === "/api/account/me") return Response.json(account);
    if (path === "/api/account/avatar" && method === "DELETE") {
      control.removals++;
      delete account.avatarAsset;
      delete account.avatarUrl;
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
      const reservation = control.reservations.at(-1);
      const id = complete[1];
      if (reservation.purpose === "avatar") {
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
  window.WebSocket = class extends EventTarget {
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
      if (request.type === "subscribe" && request.kind === "profiles") {
        // Live profile/space updates: no snapshot, and events only when the test pushes them.
        control.profilesFrame = request;
        control.pushLive = (event) => this.frame({ type: "event", id: request.id, event });
        this.frame({ type: "subscribed", id: request.id });
        return;
      }
      if (request.type === "subscribe") {
        const event =
          request.kind === "chat" || request.kind === "notifications"
            ? { type: "ready", cursor: "5" }
            : request.kind === "presence"
              ? { type: "snapshot", members: request.userIds.map((userId) => ({ userId, status: "online" })) }
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
    close() {}
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
  await created.addInitScript(`(${fixture.toString()})();`);
  await created.route("https://cdn.test/assets/**", (route) => {
    const asset = new URL(route.request().url()).pathname.split("/").pop();
    const file = stored.get(asset);
    return file
      ? route.fulfill({
          status: 200,
          body: file.body,
          contentType: file.type,
          headers: { "cache-control": "max-age=3600" },
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
  if (artifacts) await target.screenshot({ path: join(artifacts, name) });
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

const browser = await chromium.launch();
try {
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
  // Without a custom picture, the mascot holds up "change me!" and the frame opens the picker directly.
  const editPicture = dialog.getByRole("button", { name: "Change profile picture" });
  await editPicture.waitFor();
  await dialog.locator('.picture-settings-change-me[data-avatar-id="0"]').waitFor();
  assert.equal(await editPicture.getAttribute("aria-haspopup"), null);
  await page.waitForTimeout(300); // Let the sign's vector arrive.
  await shot(dialog, "pictures-profile-mascot.png");
  await dialog.locator('.picture-settings input[type="file"]').setInputFiles({
    name: "landscape-photo.png",
    mimeType: "image/png",
    buffer: landscape,
  });
  await dialog.getByText("Profile picture updated.").waitFor();
  const still = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.equal(still.purpose, "avatar");
  assert.equal(still.contentType, "image/webp");
  assert.equal(still.filename, "avatar.webp");
  assert.equal(still.width, 512);
  assert.equal(still.height, 512);
  assert.ok(still.byteSize > 0 && still.byteSize <= 1024 * 1024);
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
  await shot(dialog, "pictures-profile-uploaded.png");
  await page.keyboard.press("Escape");
  await dialog.getByRole("menu").waitFor({ state: "detached" });
  assert.ok(await dialog.isVisible(), "Escape closes the menu, not the dialog");
  await dialog.locator('.picture-settings input[type="file"]').setInputFiles({
    name: "party.gif",
    mimeType: "image/gif",
    buffer: animatedAvatar,
  });
  await page.waitForFunction(() => window.pictureFixture.completes === 2);
  const animated = await page.evaluate(() => window.pictureFixture.reservations.at(-1));
  assert.deepEqual(
    [animated.contentType, animated.filename, animated.byteSize, animated.width, animated.height],
    ["image/gif", "avatar.gif", animatedAvatar.length, 64, 64],
  );
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000002"]'));
  // Local and server errors are explained inline.
  await dialog.locator('.picture-settings input[type="file"]').setInputFiles({
    name: "huge.gif",
    mimeType: "image/gif",
    buffer: oversizedGif,
  });
  assert.ok(oversizedGif.length > 5 * 1024 * 1024);
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
    return { frames: decoder.tracks.selectedTrack.frameCount, width: image.displayWidth, height: image.displayHeight };
  }, shrunkBody.toString("base64"));
  assert.deepEqual(playback, { frames: 12, width: 512, height: 512 });
  await page.waitForFunction(loaded('.account-avatar [data-avatar-asset="upload000003"]'));
  await dialog.locator('.picture-settings input[type="file"]').setInputFiles({
    name: "photo.heic",
    mimeType: "image/heic",
    buffer: Buffer.from("not a picture this browser can decode"),
  });
  await dialog.getByText("This browser can’t open that picture. Try a PNG, JPEG, WebP or GIF.").waitFor();
  await page.evaluate(() => {
    window.pictureFixture.failNext = { status: 429, body: { error: "too many", code: "picture_rate_limited" } };
  });
  await dialog.locator('.picture-settings input[type="file"]').setInputFiles({
    name: "again.gif",
    mimeType: "image/gif",
    buffer: animatedAvatar,
  });
  await dialog.getByText("You’ve changed pictures too many times today. Try again tomorrow.").waitFor();
  await shot(dialog, "pictures-profile-error.png");
  await editPicture.click();
  await dialog.getByRole("menuitem", { name: "Remove picture" }).click();
  await dialog.getByText("Profile picture removed.").waitFor();
  await page.waitForFunction(
    `[...document.querySelectorAll(${JSON.stringify(ownAvatars)})].every((element) => !element.dataset.avatarAsset && !element.querySelector(".caper-picture") && element.dataset.avatarId === "0")`,
  );
  await dialog.locator(".picture-settings-change-me").waitFor();
  assert.equal(await editPicture.getAttribute("aria-haspopup"), null);
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
  const iconButton = manage.getByRole("button", { name: "Change space icon" });
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
  await manage.locator(".picture-settings-error").waitFor();
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
    '[...document.querySelectorAll(".caper-picture")].filter((picture) => picture.dataset.state !== "loading").every((picture) => picture.hasAttribute("data-still") && picture.querySelector("canvas")?.width <= 256 && getComputedStyle(picture.querySelector("img")).visibility === "hidden")',
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
} finally {
  await browser.close();
}
