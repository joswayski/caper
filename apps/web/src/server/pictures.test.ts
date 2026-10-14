import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { getRememberedAccount, subscribeAccount, withAccountProfile, type Account } from "../account/client.ts";
import { PictureError, preparePicture, uploadAvatar, uploadPicture } from "../account/picture-upload.ts";
import {
  gifError,
  gifSize,
  MAX_GIF_BYTES,
  pictureEdge,
  pictureErrorMessage,
  pictureFailed,
  pictureFilename,
  pictureLoaded,
  pictureMessages,
  pictureSource,
  resetPicturesForTests,
  squareCrop,
  withPicture,
  withSpaceIcon,
} from "../account/pictures.ts";
import { mentionCardPerson, withInvited } from "../chat/mentions.ts";
import type { UploadTransport } from "../chat/uploads.ts";
import Avatar from "../components/Avatar.tsx";
import SpaceIcon from "../components/SpaceIcon.tsx";

/** A GIF header (signature plus logical screen size); enough for the client's checks. */
function gif(width: number, height: number, signature = "GIF89a", size = 32) {
  const bytes = new Uint8Array(size);
  bytes.set([...signature].map((char) => char.charCodeAt(0)));
  bytes.set([width & 0xff, width >> 8, height & 0xff, height >> 8], 6);
  return bytes;
}

test("GIFs are recognised by their magic bytes and logical screen size, not their name", () => {
  assert.deepEqual(gifSize(gif(480, 270)), { width: 480, height: 270 });
  assert.deepEqual(gifSize(gif(1024, 1, "GIF87a")), { width: 1024, height: 1 });
  assert.equal(gifSize(gif(10, 10, "GIF90a")), undefined);
  assert.equal(gifSize(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0])), undefined);
  assert.equal(gifSize(gif(10, 10).subarray(0, 9)), undefined);
});

test("GIFs upload unchanged only up to 5 MiB and 1024 × 1024", () => {
  assert.equal(gifError(MAX_GIF_BYTES, { width: 1024, height: 1024 }), undefined);
  for (const [size, width, height] of [
    [MAX_GIF_BYTES + 1, 10, 10],
    [100, 1025, 10],
    [100, 10, 1025],
    [100, 0, 10],
  ])
    assert.equal(gifError(size, { width, height }), "GIFs can be up to 5 MB and 1024 × 1024.");
});

test("stills are center-cropped square and scaled to their short edge, at most 512", () => {
  assert.deepEqual(squareCrop(1200, 800), { x: 200, y: 0, side: 800 });
  assert.deepEqual(squareCrop(801, 1001), { x: 0, y: 100, side: 801 });
  assert.deepEqual(squareCrop(64, 64), { x: 0, y: 0, side: 64 });
  assert.equal(pictureEdge(4032, 3024), 512);
  assert.equal(pictureEdge(300, 900), 300);
  assert.equal(pictureEdge(512, 512), 512);
  assert.equal(pictureEdge(0, 10), 1);
  assert.equal(pictureFilename("avatar", "image/webp"), "avatar.webp");
  assert.equal(pictureFilename("spaceIcon", "image/gif"), "space-icon.gif");
  assert.equal(pictureFilename("avatar", "image/jpeg"), "avatar.jpg");
});

test("API errors read as friendly sentences", () => {
  const message = (status: number, body?: { error?: string; code?: string }, purpose = "avatar" as const) =>
    pictureErrorMessage(status, body, purpose);
  assert.equal(
    message(400, { error: "pictures must be PNG, JPEG, WebP or GIF", code: "picture_type" }),
    pictureMessages.type,
  );
  assert.equal(message(400, { error: "width and height must be 1-1024" }), pictureMessages.dimensions);
  assert.equal(message(413, { code: "picture_too_large" }), pictureMessages.serverTooLarge);
  assert.equal(message(429, { code: "picture_rate_limited" }), pictureMessages.rateLimited);
  assert.equal(message(422, { code: "picture_invalid" }), "That picture couldn’t be used. Try another one.");
  assert.equal(message(503), pictureMessages.unavailable);
  assert.equal(pictureErrorMessage(404, undefined, "spaceIcon"), pictureMessages.notOwner);
  assert.equal(message(500, { error: "internal" }), pictureMessages.failed);
});

test("the saved account's picture wins over snapshots, and removing it clears theirs", () => {
  const account: Account = {
    id: "self",
    username: "me",
    displayName: "Me",
    avatarId: 4,
    avatarAsset: "new",
    avatarUrl: "https://cdn.test/new",
  };
  const stale = {
    id: "self",
    username: "me",
    displayName: "Me",
    avatarId: 4,
    avatarAsset: "old",
    avatarUrl: "https://cdn.test/old",
  };
  assert.deepEqual(withAccountProfile(stale, account), account);
  const { avatarAsset: _asset, avatarUrl: _url, ...removed } = account;
  assert.deepEqual(withAccountProfile(stale, removed), removed);
  const other = { ...stale, id: "other" };
  assert.equal(withAccountProfile(other, account), other);
  // Without a custom picture nothing new appears: old snapshots stay key-for-key equal.
  assert.deepEqual(withPicture({ id: "a", avatarId: 1 }, {}), { id: "a", avatarId: 1 });
});

test("pictures carry through invitations and profile cards", () => {
  const invited = {
    id: "u1",
    username: "maya",
    displayName: "Maya",
    avatarId: 3,
    avatarAsset: "pic",
    avatarUrl: "https://cdn.test/pic",
  };
  assert.deepEqual(withInvited([], [invited]).at(-1), { ...invited, invited: true });
  assert.deepEqual(mentionCardPerson({ id: "u1", name: "Maya" }, [invited], undefined), {
    id: "u1",
    username: "maya",
    displayName: "Maya",
    avatarId: 3,
    avatarAsset: "pic",
    avatarUrl: "https://cdn.test/pic",
    self: false,
  });
  const author = { id: "u2", name: "Sam", avatarAsset: "sam", avatarUrl: "https://cdn.test/sam" };
  assert.equal(mentionCardPerson(author, [], undefined).avatarUrl, "https://cdn.test/sam");
});

test("an icon change replaces the space's icon fields, and a removal drops them", () => {
  const space = {
    id: "s",
    name: "Studio",
    ownerId: "o",
    iconAsset: "old",
    iconUrl: "https://cdn.test/old",
    demo: false,
  };
  assert.deepEqual(
    withSpaceIcon(space, { id: "s", name: "Studio", ownerId: "o", iconAsset: "new", iconUrl: "https://cdn.test/new" }),
    {
      ...space,
      iconAsset: "new",
      iconUrl: "https://cdn.test/new",
    },
  );
  assert.deepEqual(withSpaceIcon(space, { id: "s", name: "Studio", ownerId: "o" }), {
    id: "s",
    name: "Studio",
    ownerId: "o",
    demo: false,
  });
});

test("pictures are cached by asset: a re-signed URL reuses the loaded copy, a failure retries with a fresh URL", () => {
  resetPicturesForTests();
  assert.equal(pictureSource(undefined, "https://cdn.test/a?sig=1"), undefined);
  assert.equal(pictureSource("a", "javascript:alert(1)"), undefined);
  assert.equal(pictureSource("a", "https://cdn.test/a?sig=1"), "https://cdn.test/a?sig=1");
  pictureLoaded("a", "https://cdn.test/a?sig=1");
  assert.equal(pictureSource("a", "https://cdn.test/a?sig=2"), "https://cdn.test/a?sig=1");
  // The loaded copy expired: the fresh URL is tried, the failed one never again.
  pictureFailed("a", "https://cdn.test/a?sig=1");
  assert.equal(pictureSource("a", "https://cdn.test/a?sig=1"), undefined);
  assert.equal(pictureSource("a", "https://cdn.test/a?sig=2"), "https://cdn.test/a?sig=2");
  // A new picture is a new asset and never reuses the old one.
  assert.equal(pictureSource("b", "https://cdn.test/b?sig=1"), "https://cdn.test/b?sig=1");
  for (let index = 0; index < 600; index++) pictureLoaded(`bulk${index}`, `https://cdn.test/${index}`);
  assert.equal(pictureSource("bulk0", "https://cdn.test/fresh"), "https://cdn.test/fresh");
  assert.equal(pictureSource("bulk599", "https://cdn.test/fresh"), "https://cdn.test/599");
  resetPicturesForTests();
});

test("avatars lay a custom picture over the mascot, which stays for loading and failure", () => {
  resetPicturesForTests();
  const markup = renderToStaticMarkup(
    createElement(Avatar, {
      person: { id: "u", avatarId: 7, avatarAsset: "pic", avatarUrl: "https://cdn.test/pic.gif" },
      name: "Maya",
    }),
  );
  assert.match(markup, /data-avatar-id="7"/);
  assert.match(markup, /background-image:url\(&#x27;\/images\/avatars\/v3\/7\.svg&#x27;\)/);
  assert.match(
    markup,
    // No shape of its own: it inherits the container's rounded square (--radius-avatar).
    /<span class="caper-picture" data-state="loading"><img src="https:\/\/cdn\.test\/pic\.gif" alt=""/,
  );
  assert.doesNotMatch(renderToStaticMarkup(createElement(Avatar, { person: { avatarId: 7 }, name: "M" })), /<img/);
  // Not an http(s) URL: the mascot alone.
  const unsafe = renderToStaticMarkup(
    createElement(Avatar, {
      person: { avatarId: 7, avatarAsset: "x", avatarUrl: "data:image/gif;base64,AAAA" },
      name: "M",
    }),
  );
  assert.doesNotMatch(unsafe, /<img/);
  pictureFailed("pic", "https://cdn.test/pic.gif");
  const failed = renderToStaticMarkup(
    createElement(Avatar, {
      person: { avatarId: 7, avatarAsset: "pic", avatarUrl: "https://cdn.test/pic.gif" },
      name: "M",
    }),
  );
  assert.doesNotMatch(failed, /<img/);
  const icon = renderToStaticMarkup(
    createElement(SpaceIcon, { space: { name: "studio", iconAsset: "icon", iconUrl: "https://cdn.test/icon" } }),
  );
  assert.match(icon, /^<span aria-hidden="true">S<\/span><span class="caper-picture"/);
  resetPicturesForTests();
});

test("GIFs are prepared unchanged; oversized GIFs and undecodable files fail before any upload", async () => {
  const animated = new Blob([gif(480, 480)], { type: "" });
  const prepared = await preparePicture(animated);
  assert.equal(prepared.blob, animated);
  assert.deepEqual(
    { ...prepared, blob: undefined },
    { blob: undefined, contentType: "image/gif", width: 480, height: 480 },
  );
  await assert.rejects(preparePicture(new Blob([gif(2048, 10)])), (error: unknown) => {
    return error instanceof PictureError && error.message === "GIFs can be up to 5 MB and 1024 × 1024.";
  });
  // Node has no image decoder, like a browser given HEIC it can't open.
  await assert.rejects(preparePicture(new Blob([new Uint8Array(64)], { type: "image/heic" })), (error: unknown) => {
    return error instanceof PictureError && error.message === pictureMessages.undecodable;
  });
});

function transport(responses: Record<string, Response[]>) {
  const calls: { path: string; init?: RequestInit }[] = [];
  const puts: { url: string; headers: Record<string, string>; size: number }[] = [];
  const value: UploadTransport = {
    fetch: async (input, init) => {
      const path = String(input);
      calls.push({ path, init });
      const next = responses[path]?.shift();
      return next ?? Response.json({ error: "unexpected" }, { status: 500 });
    },
    put: async (url, headers, body, progress) => {
      puts.push({ url, headers, size: body.size });
      progress(1);
    },
    sleep: async () => {},
  };
  return { value, calls, puts };
}

const reservation = () =>
  Response.json(
    {
      id: "asset123",
      kind: "image",
      upload: {
        method: "PUT",
        url: "https://r2.test/put",
        headers: { "content-type": "image/gif", "content-disposition": "inline" },
      },
    },
    { status: 201 },
  );

test("a space icon reserves with its purpose and space, PUTs the signed headers, and retries an unfinished complete", async () => {
  const fake = transport({
    "/api/assets": [reservation()],
    "/api/assets/asset123/complete": [
      Response.json({ error: "upload not finished" }, { status: 409 }),
      Response.json({
        id: "asset123",
        space: { id: "space1234567", name: "Studio", ownerId: "o", iconAsset: "asset123" },
      }),
    ],
  });
  const picture = { blob: new Blob([gif(64, 64)]), contentType: "image/gif", width: 64, height: 64 };
  const progress: number[] = [];
  const body = await uploadPicture(
    picture,
    { purpose: "spaceIcon", spaceId: "space1234567" },
    (value) => progress.push(value),
    fake.value,
  );
  assert.deepEqual(JSON.parse(String(fake.calls[0].init?.body)), {
    purpose: "spaceIcon",
    spaceId: "space1234567",
    filename: "space-icon.gif",
    contentType: "image/gif",
    byteSize: 32,
    width: 64,
    height: 64,
  });
  assert.deepEqual(fake.puts, [
    { url: "https://r2.test/put", headers: { "content-type": "image/gif", "content-disposition": "inline" }, size: 32 },
  ]);
  assert.equal(fake.calls.filter((call) => call.path.endsWith("/complete")).length, 2);
  assert.equal((body.space as { iconAsset: string }).iconAsset, "asset123");
  assert.equal(progress.at(-1), 1);
});

test("rejected pictures explain why, and a failed complete cancels the reservation", async () => {
  const picture = { blob: new Blob([gif(64, 64)]), contentType: "image/gif", width: 64, height: 64 };
  const tooLarge = transport({
    "/api/assets": [Response.json({ error: "picture is too large", code: "picture_too_large" }, { status: 413 })],
  });
  await assert.rejects(
    uploadPicture(picture, { purpose: "avatar" }, () => {}, tooLarge.value),
    {
      message: pictureMessages.serverTooLarge,
    },
  );
  const unavailable = transport({ "/api/assets": [Response.json({ error: "uploads unavailable" }, { status: 503 })] });
  await assert.rejects(
    uploadPicture(picture, { purpose: "avatar" }, () => {}, unavailable.value),
    (error: unknown) => {
      return error instanceof PictureError && error.status === 503 && error.message === pictureMessages.unavailable;
    },
  );
  const invalid = transport({
    "/api/assets": [reservation()],
    "/api/assets/asset123/complete": [
      Response.json({ error: "invalid picture", code: "picture_invalid" }, { status: 422 }),
    ],
  });
  await assert.rejects(
    uploadPicture(picture, { purpose: "avatar" }, () => {}, invalid.value),
    {
      message: "That picture couldn’t be used. Try another one.",
    },
  );
  assert.ok(invalid.calls.some((call) => call.path === "/api/assets/asset123/cancel"));
});

test("a new profile picture updates the remembered account for every subscriber", async (t) => {
  const account = {
    id: "self",
    username: "me",
    displayName: "Me",
    avatarId: 9,
    avatarAsset: "asset123",
    avatarUrl: "https://cdn.test/me",
  };
  const fake = transport({
    "/api/assets": [reservation()],
    "/api/assets/asset123/complete": [Response.json({ id: "asset123", kind: "image", account })],
  });
  let notified = 0;
  t.onTestFinished(subscribeAccount(() => notified++));
  const saved = await uploadAvatar(new Blob([gif(128, 128)]), () => {}, fake.value);
  assert.deepEqual(saved, account);
  assert.deepEqual(getRememberedAccount(), account);
  assert.ok(notified > 0);
});
