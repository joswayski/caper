import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { inflateSync } from "node:zlib";
import {
  startFixture,
  fixtureHost,
  benchFormattingChannel,
  fixtureIDs as ids,
} from "../scripts/native-parity-fixture.mjs";

async function setup(t) {
  const fixture = await startFixture({ port: 0, gatewayPort: 0 });
  t.onTestFinished(() => fixture.close());
  const base = `http://127.0.0.1:${fixture.port}`;
  const request = async (path, { auth = false, body, ...init } = {}) => {
    const response = await fetch(base + path, {
      ...init,
      headers: {
        ...(auth ? { authorization: `Bearer ${auth === true ? "fixture-owner-token" : auth}` } : {}),
        ...(body === undefined ? {} : { "content-type": "application/json" }),
        ...init.headers,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const value = response.status === 204 ? undefined : await response.json();
    return { response, value };
  };
  return { fixture, base, request };
}

function socket(url, options) {
  const ws = new WebSocket(url, options);
  const queued = [];
  const waiting = [];
  ws.addEventListener("message", ({ data }) => {
    const value = JSON.parse(data);
    const resolve = waiting.shift();
    if (resolve) resolve(value);
    else queued.push(value);
  });
  const next = () =>
    queued.length ? Promise.resolve(queued.shift()) : new Promise((resolve) => waiting.push(resolve));
  return {
    ws,
    next,
    opened: new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    }),
  };
}

test("email code auth and validated profile persist across account and member reads", async (t) => {
  const { request } = await setup(t);
  assert.equal((await request("/api/account/me")).response.status, 401);
  assert.equal(
    (await request("/api/auth/email/request", { method: "POST", body: { email: "bad" } })).response.status,
    400,
  );
  const challenge = await request("/api/auth/email/request", { method: "POST", body: { email: "owner@example.test" } });
  const wrong = await request("/api/auth/email/verify", {
    method: "POST",
    body: { challengeId: challenge.value.challengeId, code: "WRONG" },
  });
  assert.equal(wrong.response.status, 401);
  assert.equal(wrong.value.attemptsRemaining, 2);
  const verified = await request("/api/auth/email/verify", {
    method: "POST",
    body: { challengeId: challenge.value.challengeId, code: "ABC234" },
  });
  assert.equal(verified.value.account.id, ids.owner);
  assert.equal(
    (
      await request("/api/account/profile", {
        method: "POST",
        auth: true,
        body: { username: "no-dashes", displayName: "Owner" },
      })
    ).response.status,
    400,
  );
  const saved = await request("/api/account/profile", {
    method: "POST",
    auth: true,
    body: { username: " New_Owner ", displayName: " New Name " },
  });
  assert.deepEqual(saved.value, { id: ids.owner, username: "new_owner", displayName: "New Name", avatarId: 0 });
  assert.deepEqual((await request("/api/account/me", { auth: true })).value, saved.value);
  const detail = await request(`/api/spaces/${ids.space}`, { auth: true });
  assert.equal(detail.value.members.find(({ id }) => id === ids.owner).displayName, "New Name");
  assert.deepEqual(
    detail.value.members.map((member) => member.avatarId),
    [0, 31, 799],
  );
  const history = await request(`/api/chat/channels/${ids.general}/messages`, { auth: true });
  assert.deepEqual(
    history.value.messages.map((message) => message.author.avatarId),
    [0, 31, 31, 799],
  );
});

test("profile drafts stage privately, then publish names and avatar together exactly once", async (t) => {
  const { fixture, request } = await setup(t);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`, {
    headers: { authorization: "Bearer fixture-owner-token" },
  });
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, "hello");
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "profiles", kind: "profiles" }));
  assert.equal((await stream.next()).type, "subscribed");
  const noUpdate = async () => {
    // An ordered socket barrier: any premature/duplicate event arrives first.
    stream.ws.send(JSON.stringify({ type: "heartbeat" }));
    assert.equal((await stream.next()).type, "heartbeat");
  };
  const original = (await request("/api/account/me", { auth: true })).value;
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true } });
  // Real one-pixel GIF bytes, served unchanged by this disposable fixture.
  const bytes = Buffer.from("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7", "base64");
  const reserved = await request("/api/assets", {
    auth: true,
    method: "POST",
    body: {
      purpose: "avatar",
      filename: "avatar.gif",
      contentType: "image/gif",
      byteSize: bytes.length,
      width: 1,
      height: 1,
    },
  });
  const asset = reserved.value.id;
  const input = {
    username: "new_owner",
    displayName: "New Name",
    avatar: {
      avatarId: 799,
      avatarAsset: asset,
      expectedAvatarId: 0,
      expectedAvatarAsset: null,
    },
  };
  assert.equal(
    (await request("/api/account/profile", { auth: true, method: "POST", body: input })).response.status,
    404,
    "uncompleted assets cannot publish even valid names",
  );
  assert.equal(
    (await request(`/api/assets/${asset}/complete?draft=true`, { auth: true, method: "POST" })).response.status,
    422,
  );
  assert.equal((await fetch(reserved.value.upload.url, { method: "PUT", body: bytes })).status, 204);
  const staged = await request(`/api/assets/${asset}/complete?draft=true`, { auth: true, method: "POST" });
  assert.equal(staged.response.status, 200);
  assert.equal(staged.value.id, asset);
  assert.deepEqual(staged.value.account, original);
  assert.deepEqual((await request("/api/account/me", { auth: true })).value, original);
  assert.deepEqual(
    (await request(`/api/assets/${asset}/complete`, { auth: true, method: "POST" })).value.account,
    original,
    "a completion retry without draft=true must not publish a previously staged asset",
  );
  await noUpdate();
  const history = (await request("/api/account/avatar/history", { auth: true })).value;
  assert.equal(history.pictures[0].avatarAsset, asset);
  assert.equal(history.nextCursor, null);
  assert.deepEqual((await request(`/api/account/avatar/history?cursor=${asset}`, { auth: true })).value, {
    pictures: [],
    nextCursor: null,
  });
  assert.deepEqual((await request("/api/account/avatar/history", { auth: "fixture-member-token" })).value.pictures, []);
  assert.deepEqual(Buffer.from(await (await fetch(history.pictures[0].avatarUrl)).arrayBuffer()), bytes);

  const saved = await request("/api/account/profile", { auth: true, method: "POST", body: input });
  assert.equal(saved.response.status, 200);
  assert.deepEqual(saved.value, {
    ...original,
    username: "new_owner",
    displayName: "New Name",
    avatarId: 799,
    avatarAsset: asset,
    avatarUrl: history.pictures[0].avatarUrl,
  });
  assert.deepEqual((await stream.next()).event, { type: "profile.updated", person: saved.value });
  await noUpdate();
  const member = (await request(`/api/spaces/${ids.space}`, { auth: true })).value.members.find(
    (person) => person.id === ids.owner,
  );
  assert.equal(member.displayName, "New Name");
  assert.equal(member.avatarAsset, asset);

  for (const avatar of [input.avatar, { ...input.avatar, expectedAvatarId: 799 }]) {
    const stale = await request("/api/account/profile", {
      auth: true,
      method: "POST",
      body: { ...input, displayName: "Must not apply", avatar },
    });
    assert.equal(stale.response.status, 409);
    assert.equal(stale.value.code, "profile_changed");
  }
  assert.deepEqual((await request("/api/account/me", { auth: true })).value, saved.value);
  await noUpdate();

  const namesOnly = await request("/api/account/profile", {
    auth: true,
    method: "POST",
    body: { username: "new_owner", displayName: "Name only" },
  });
  assert.equal(namesOnly.value.avatarAsset, asset, "omitting avatar preserves the picture");
  assert.equal((await stream.next()).event.person.displayName, "Name only");
  const removed = await request("/api/account/profile", {
    auth: true,
    method: "POST",
    body: { ...input, avatar: { avatarId: 799, avatarAsset: null, expectedAvatarId: 799, expectedAvatarAsset: asset } },
  });
  assert.equal(removed.value.avatarAsset, null);
  assert.equal((await stream.next()).event.person.avatarAsset, null);
  assert.equal((await request("/api/account/avatar/history", { auth: true })).value.pictures[0].avatarAsset, asset);
  const restored = await request("/api/account/profile", {
    auth: true,
    method: "POST",
    body: { ...input, avatar: { ...input.avatar, expectedAvatarId: 799 } },
  });
  assert.equal(restored.value.avatarAsset, asset, "Use picture can select completed history without another upload");
  assert.equal((await stream.next()).event.person.avatarAsset, asset);
  await noUpdate();
});

test("profile publication rejects invalid and foreign pictures; legacy completion applies only once", async (t) => {
  const { request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true } });
  const original = (await request("/api/account/me", { auth: true })).value;
  const reserved = await request("/api/assets", {
    auth: "fixture-member-token",
    method: "POST",
    body: {
      purpose: "avatar",
      filename: "avatar.gif",
      contentType: "image/gif",
      byteSize: 3,
      width: 1,
      height: 1,
    },
  });
  await fetch(reserved.value.upload.url, { method: "PUT", body: new Uint8Array([1, 2, 3]) });
  const path = `/api/assets/${reserved.value.id}/complete`;
  assert.equal((await request(path, { auth: true, method: "POST" })).response.status, 404);
  const legacy = await request(path, { auth: "fixture-member-token", method: "POST" });
  assert.equal(legacy.value.account.id, ids.member);
  assert.equal(legacy.value.account.avatarAsset, reserved.value.id);
  assert.deepEqual(
    (await request("/api/account/me", { auth: true })).value,
    original,
    "another account's upload never changes the owner",
  );
  const avatar = { avatarId: 1, avatarAsset: reserved.value.id, expectedAvatarId: 0, expectedAvatarAsset: null };
  for (const [candidate, status] of [
    [avatar, 404],
    [{ ...avatar, avatarId: 800 }, 400],
    [{ ...avatar, expectedAvatarId: "0" }, 400],
  ]) {
    assert.equal(
      (
        await request("/api/account/profile", {
          auth: true,
          method: "POST",
          body: {
            username: "untouched",
            displayName: "Untouched",
            avatar: candidate,
          },
        })
      ).response.status,
      status,
    );
  }
  assert.deepEqual((await request("/api/account/me", { auth: true })).value, original);
  const removed = await request("/api/account/profile", {
    auth: "fixture-member-token",
    method: "POST",
    body: {
      username: "maya",
      displayName: "Maya",
      avatar: { avatarId: 31, avatarAsset: null, expectedAvatarId: 31, expectedAvatarAsset: reserved.value.id },
    },
  });
  assert.equal(removed.response.status, 200);
  assert.equal(
    (await request(path, { auth: "fixture-member-token", method: "POST" })).value.account.avatarAsset,
    null,
    "retrying old completion cannot undo a later selection",
  );
});

test("persistent navigation failure survives prefetch until explicitly cleared", async (t) => {
  const { request } = await setup(t);
  const path = `/api/spaces/${ids.space}`;
  await request("/__fixture/control", {
    method: "POST",
    body: { failure: { path, method: "GET", status: 503, persistent: true } },
  });
  assert.equal((await request(path, { auth: true })).response.status, 503, "hover read fails");
  assert.equal((await request(path, { auth: true })).response.status, 503, "click read still fails");
  await request("/__fixture/control", { method: "POST", body: { clearFailures: true } });
  assert.equal((await request(path, { auth: true })).response.status, 200, "explicit retry can succeed");
  await request("/__fixture/control", { method: "POST", body: { failure: { path, status: 502 } } });
  assert.equal((await request(path, { auth: true })).response.status, 502);
  assert.equal((await request(path, { auth: true })).response.status, 200, "ordinary failures remain one-shot");
});

test("space invitation consent and channel grants enforce distinct access contracts", async (t) => {
  const { request } = await setup(t);
  assert.equal(
    (await request("/api/spaces", { method: "POST", auth: true, body: { name: "bad\nname" } })).response.status,
    400,
  );
  const created = await request("/api/spaces", { method: "POST", auth: true, body: { name: "  Field Notes  " } });
  assert.equal(created.response.status, 201);
  assert.equal(created.value.name, "Field Notes");
  const spaceId = created.value.id;
  const added = await request(`/api/spaces/${spaceId}/members`, {
    method: "POST",
    auth: true,
    body: { username: "maya" },
  });
  assert.equal(added.response.status, 201);
  assert.equal(
    (await request(`/api/spaces/${spaceId}/members`, { method: "POST", auth: true, body: { username: "maya" } }))
      .response.status,
    409,
  );
  assert.equal((await request(`/api/spaces/${spaceId}/invitations`, { auth: true })).value.members[0].id, ids.member);
  assert.equal((await request(`/api/spaces/${spaceId}`, { auth: "fixture-member-token" })).response.status, 404);
  const pending = await request("/api/spaces", { auth: "fixture-member-token" });
  assert.deepEqual(
    pending.value.invitations.map(({ id }) => id),
    [spaceId],
  );
  assert.deepEqual(pending.value.invitations[0].inviter, { username: "fixture_owner", displayName: "Fixture Owner" });
  assert.ok(!pending.value.spaces.some(({ id }) => id === spaceId));
  const channel = await request(`/api/spaces/${spaceId}/channels`, {
    method: "POST",
    auth: true,
    body: { name: "private-notes", private: true },
  });
  assert.equal(channel.response.status, 201);
  assert.equal(
    (
      await request(`/api/spaces/${spaceId}/channels`, {
        method: "POST",
        auth: true,
        body: { name: "private-notes", private: false },
      })
    ).response.status,
    409,
  );
  assert.equal(
    (
      await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, {
        method: "POST",
        auth: true,
        body: { username: "alex" },
      })
    ).response.status,
    404,
  );
  assert.equal(
    (
      await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, {
        method: "POST",
        auth: true,
        body: { username: "maya" },
      })
    ).response.status,
    404,
  );
  assert.equal(
    (await request(`/api/spaces/${spaceId}/invitation`, { method: "POST", auth: "fixture-member-token" })).response
      .status,
    200,
  );
  assert.equal(
    (await request(`/api/spaces/${spaceId}/invitation`, { method: "POST", auth: "fixture-member-token" })).response
      .status,
    404,
  );
  assert.equal(
    (
      await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, {
        method: "POST",
        auth: true,
        body: { username: "maya" },
      })
    ).response.status,
    201,
  );
  assert.equal(
    (await request(`/api/spaces/${spaceId}/members/${ids.member}`, { method: "DELETE", auth: true })).response.status,
    204,
  );
  assert.equal(
    (await request(`/api/spaces/${spaceId}/channels/${channel.value.id}/members`, { auth: true })).value.members.some(
      ({ id }) => id === ids.member,
    ),
    false,
  );
  assert.equal((await request(`/api/spaces/${spaceId}`, { auth: "fixture-member-token" })).response.status, 404);
});

test("management can invite a nonmember without bypassing removed-member cooldown", async (t) => {
  const { request } = await setup(t);
  const root = `/api/spaces/${ids.space}`;
  assert.equal((await request(`${root}/members/${ids.member}`, { method: "DELETE", auth: true })).response.status, 204);
  const reinvite = await request(`${root}/members`, { method: "POST", auth: true, body: { username: "maya" } });
  assert.equal(reinvite.response.status, 409);
  assert.match(reinvite.value.error, /cooldown/);
  assert.deepEqual((await request(`${root}/invitations`, { auth: true })).value.members, []);

  const invited = await request(`${root}/members`, { method: "POST", auth: true, body: { username: "sam" } });
  assert.equal(invited.response.status, 201);
  assert.equal(invited.value.id, ids.invitee);
  assert.deepEqual(
    (await request(`${root}/invitations`, { auth: true })).value.members.map((member) => member.username),
    ["sam"],
  );
  assert.deepEqual(
    (await request(root, { auth: true })).value.members.map((member) => member.id),
    [ids.owner, ids.other],
    "a pending invitation must not grant membership",
  );
  assert.equal(
    (
      await request(`${root}/channels/${ids.private}/members`, {
        method: "POST",
        auth: true,
        body: { username: "sam" },
      })
    ).response.status,
    404,
    "a pending invitee cannot receive private channel membership",
  );
});

test("public preview stays readable without participation; private consent grants and joins together", async (t) => {
  const { request } = await setup(t);
  const publicRoot = `/api/spaces/${ids.space}/channels/${ids.design}`;
  const privateRoot = `/api/spaces/${ids.space}/channels/${ids.private}`;
  const member = { auth: "fixture-member-token" };
  assert.deepEqual((await request(`/api/spaces/${ids.space}`, member)).value.channelMembers, {
    [ids.private]: [ids.owner, ids.member],
  });
  assert.equal((await request(`${publicRoot}/membership`, { ...member, method: "DELETE" })).response.status, 204);
  assert.equal((await request(`/api/chat/channels/${ids.design}/messages`, member)).response.status, 200);
  assert.equal((await request(`/api/channels/${ids.design}/media/status`, member)).response.status, 404);
  assert.equal(
    (
      await request(`/api/chat/channels/${ids.design}/messages`, {
        ...member,
        method: "POST",
        body: { text: "not joined" },
      })
    ).response.status,
    404,
  );
  assert.equal(
    (await request(`/api/spaces/${ids.space}`, member)).value.channels.find((channel) => channel.id === ids.design)
      .joined,
    false,
  );
  assert.equal((await request(`${publicRoot}/membership`, { ...member, method: "POST" })).value.joined, true);

  assert.equal(
    (await request(`${privateRoot}/members/${ids.member}`, { auth: true, method: "DELETE" })).response.status,
    204,
  );
  // Reset only the disposable fixture's invitation tombstone to exercise a new invitation.
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  const created = await request(`/api/spaces/${ids.space}/channels`, {
    auth: true,
    method: "POST",
    body: { name: "private-consent", private: true },
  });
  const root = `/api/spaces/${ids.space}/channels/${created.value.id}`;
  assert.equal(
    (await request(`${root}/members`, { auth: true, method: "POST", body: { username: "maya" } })).response.status,
    201,
  );
  const before = (await request(`/api/spaces/${ids.space}`, member)).value;
  assert.ok(!before.channels.some((channel) => channel.id === created.value.id));
  assert.equal(before.channelMembers[created.value.id], undefined, "private reader lists stay hidden until acceptance");
  assert.equal(before.channelInvitations[0].inviter.username, "fixture_owner");
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 404);
  assert.equal((await request(`${root}/membership`, { ...member, method: "POST" })).response.status, 404);
  assert.equal((await request(`${root}/invitation`, { ...member, method: "POST" })).value.joined, true);
  assert.deepEqual((await request(`/api/spaces/${ids.space}`, member)).value.channelMembers[created.value.id], [
    ids.owner,
    ids.member,
  ]);
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 200);
  assert.equal((await request(`${root}/membership`, { ...member, method: "DELETE" })).response.status, 204);
  assert.equal((await request(`/api/spaces/${ids.space}`, member)).value.channelMembers[created.value.id], undefined);
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 404);
});

test("send is idempotent, conflicts on changed payload, and history uses stable head cursor pagination", async (t) => {
  const { request } = await setup(t);
  const session = await request("/api/chat/session", { method: "POST", body: { name: "Guest" } });
  const send = (clientMessageId, text) =>
    request(`/api/chat/channels/${ids.demo}/messages`, {
      method: "POST",
      headers: { "x-caper-chat-token": session.value.token },
      body: { clientMessageId, text },
    });
  const id = "AB12CD34-EF56-4789-8ABC-DEF012345678";
  const canonicalId = "ab12cd34-ef56-4789-8abc-def012345678";
  const first = await send(id, "  preserved  ");
  assert.equal(first.response.status, 200);
  assert.equal(first.value.content.text, "  preserved  ");
  assert.equal(
    first.value.clientMessageId,
    canonicalId,
    "match Rust UUID serialization instead of echoing Swift casing",
  );
  assert.equal((await send(id, "  preserved  ")).value.id, first.value.id);
  assert.equal(
    (await send(canonicalId, "  preserved  ")).value.id,
    first.value.id,
    "UUID casing cannot create a second message",
  );
  assert.equal((await send(canonicalId, "changed")).response.status, 409);
  assert.equal((await send("not-a-uuid", "text")).response.status, 400);
  for (let index = 0; index < 48; index++) await send(randomUUID(), `message ${index}`);
  const latest = await request(`/api/chat/channels/${ids.demo}/messages`);
  assert.equal(latest.value.messages.length, 50);
  assert.equal(latest.value.hasMore, true);
  assert.equal(latest.value.cursor, "53");
  const older = await request(`/api/chat/channels/${ids.demo}/messages?before=${latest.value.messages[0].seq}`);
  assert.equal(older.value.cursor, latest.value.cursor, "older pages retain the committed channel head");
  assert.ok(older.value.messages.every((message) => BigInt(message.seq) < BigInt(latest.value.messages[0].seq)));
  assert.equal((await request(`/api/chat/channels/${ids.demo}/messages?before=-1`)).response.status, 400);
});

test("held sends remain absent from history and gateway until released, then confirm once", async (t) => {
  const { fixture, request } = await setup(t);
  const path = `/api/chat/channels/${ids.demo}/messages`;
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  await stream.next(); // hello
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "chat", kind: "chat", channelId: ids.demo, after: "4" }));
  await stream.next(); // ready
  await stream.next(); // subscribed
  const events = [];
  stream.ws.addEventListener("message", ({ data }) => {
    const frame = JSON.parse(data);
    if (frame.event?.type === "message.created") events.push(frame.event.message);
  });
  const session = await request("/api/chat/session", { method: "POST", body: { name: "Pending Sender" } });
  await request("/__fixture/control", { method: "POST", body: { holdSends: true } });
  const options = {
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { clientMessageId: randomUUID(), text: "Hello" },
  };
  const sending = request(path, options);
  await vi.waitFor(async () => {
    const control = await request("/__fixture/control", { method: "POST", body: {} });
    assert.equal(control.value.heldSends, 1);
  });
  assert.equal((await request(path)).value.messages.length, 4);
  assert.deepEqual(events, [], "The pending row must not be a gateway confirmation");

  await request("/__fixture/control", { method: "POST", body: { holdSends: false } });
  const sent = await sending;
  assert.equal(sent.response.status, 200);
  assert.equal((await stream.next()).event.message.id, sent.value.id);
  const retry = await request(path, options);
  assert.equal(retry.value.id, sent.value.id);
  assert.equal(
    (await request(path)).value.messages.filter((message) => message.clientMessageId === options.body.clientMessageId)
      .length,
    1,
  );
  assert.equal(events.length, 1);
});

test("loopback WebSocket replays, delivers live messages/typing/presence, failures, and disconnects", async (t) => {
  const { fixture, base, request } = await setup(t);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, "hello");
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "chat", kind: "chat", channelId: ids.demo, after: "3" }));
  const replay = await stream.next();
  assert.equal(replay.event.message.seq, "4");
  assert.deepEqual((await stream.next()).event, { type: "ready", cursor: "4" });
  assert.equal((await stream.next()).type, "subscribed");

  const session = await request("/api/chat/session", { method: "POST", body: { name: "Socket Guest" } });
  const sent = await request(`/api/chat/channels/${ids.demo}/messages`, {
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { clientMessageId: "AB12CD34-EF56-4789-8ABC-DEF012345678", text: "live" },
  });
  const liveMessage = (await stream.next()).event.message;
  assert.equal(liveMessage.id, sent.value.id);
  assert.equal(liveMessage.clientMessageId, "ab12cd34-ef56-4789-8abc-def012345678");
  stream.ws.send(
    JSON.stringify({
      type: "command",
      id: "BC23DE45-FA67-489A-9BCD-EF0123456789",
      issuedAt: Date.now(),
      method: "typing",
      channelId: ids.demo,
      chatToken: session.value.token,
      body: { typing: true },
    }),
  );
  assert.equal((await stream.next()).event.type, "typing.updated");
  const result = await stream.next();
  assert.equal(result.status, 204);
  assert.equal(result.id, "bc23de45-fa67-489a-9bcd-ef0123456789", "command results also serialize a Rust UUID");

  stream.ws.send(
    JSON.stringify({
      type: "subscribe",
      id: "presence",
      kind: "presence",
      spaceId: ids.space,
      userIds: [ids.owner, ids.other],
    }),
  );
  assert.deepEqual(
    (await stream.next()).event.members.map(({ status }) => status),
    ["online", "idle"],
  );
  assert.equal((await stream.next()).type, "subscribed");
  stream.ws.send(
    JSON.stringify({ type: "command", id: randomUUID(), issuedAt: Date.now(), method: "media.join", body: {} }),
  );
  assert.equal((await stream.next()).status, 503);

  await request("/__fixture/control", {
    method: "POST",
    body: { failure: { path: "/api/account/me", status: 418, error: "planned" } },
  });
  assert.equal((await request("/api/account/me", { auth: true })).response.status, 418);
  const closed = new Promise((resolve) => stream.ws.addEventListener("close", resolve, { once: true }));
  await fetch(`${base}/__fixture/control`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ disconnect: true }),
  });
  await closed;
});

test("spectator rosters update and revoke without granting capture or account-channel access", async (t) => {
  const { fixture, request } = await setup(t);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, "hello");
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "private", kind: "media", channelId: ids.private }));
  assert.equal((await stream.next()).status, 403);
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "demo", kind: "media" }));
  assert.deepEqual((await stream.next()).event, { type: "snapshot", revision: 0, participants: [] });
  assert.equal((await stream.next()).type, "subscribed");
  const participant = { id: "guest-voice", name: "TEST FIXTURE Guest", muted: true, deafened: false };
  await request("/__fixture/control", {
    method: "POST",
    body: { media: { sessionStartedAt: 12345, participants: [{ ...participant, tracks: ["must not leak"] }] } },
  });
  assert.deepEqual(await stream.next(), {
    type: "event",
    id: "demo",
    event: { type: "snapshot", revision: 1, sessionStartedAt: 12345, participants: [participant] },
  });
  await request("/__fixture/control", {
    method: "POST",
    body: { media: { participants: [{ ...participant, muted: false }] } },
  });
  assert.equal((await stream.next()).event.sessionStartedAt, 12345, "roster changes retain the shared session start");
  await request("/__fixture/control", { method: "POST", body: { media: { participants: [] } } });
  assert.deepEqual((await stream.next()).event, {
    type: "snapshot",
    revision: 3,
    sessionStartedAt: null,
    participants: [],
  });
  await request("/__fixture/control", { method: "POST", body: { mediaAccessDenied: {} } });
  assert.equal((await stream.next()).status, 403);
  await request("/__fixture/control", {
    method: "POST",
    body: { media: { sessionStartedAt: 67890, participants: [participant] } },
  });
  stream.ws.send(JSON.stringify({ type: "heartbeat" }));
  assert.equal((await stream.next()).type, "heartbeat", "revocation removed the spectator subscription");
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "retry", kind: "media" }));
  assert.equal((await stream.next()).status, 403);
  await request("/__fixture/control", { method: "POST", body: { mediaAccessDenied: { denied: false } } });
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "restored", kind: "media" }));
  assert.deepEqual(await stream.next(), {
    type: "event",
    id: "restored",
    event: { type: "snapshot", revision: 4, sessionStartedAt: 67890, participants: [participant] },
  });
  assert.equal((await stream.next()).type, "subscribed");
  assert.equal((await request("/api/media/join", { method: "POST", body: {} })).response.status, 503);
});

test("DM reaction fixture advances stream and read heads without changing message sequences", async (t) => {
  const { request } = await setup(t);
  await request("/api/dms", { auth: true, method: "POST", body: { username: "fixture_alex" } });
  const session = await request("/api/chat/session", { auth: true, method: "POST", body: { name: "Fixture Owner" } });
  const root = `/api/chat/channels/${ids.direct}/messages`;
  const sent = await request(root, {
    auth: true,
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { clientMessageId: randomUUID(), text: "TEST FIXTURE — reactions" },
  });
  await request("/__fixture/control", {
    method: "POST",
    body: { incomingReaction: { channelId: ids.direct, messageId: sent.value.id, emoji: "🎉" } },
  });
  const reactionPath = `${root}/${sent.value.id}/reactions`;
  const mutation = {
    auth: true,
    method: "PUT",
    headers: { "x-caper-chat-token": session.value.token },
    body: { emoji: "🎉", active: true },
  };
  const added = await request(reactionPath, mutation);
  assert.equal(added.value.seq, "3");
  assert.deepEqual(added.value.reactions, [{ emoji: "🎉", authorIds: [ids.other, ids.owner] }]);
  assert.deepEqual((await request(reactionPath, mutation)).value, added.value, "no-op does not allocate a sequence");
  const history = (await request(root, { auth: true })).value;
  assert.equal(history.cursor, "3");
  assert.equal(history.messages[0].seq, "1");
  assert.equal(history.messages[0].reactionSeq, "3");
  await request(`/api/dms/${ids.direct}/read`, { auth: true, method: "POST", body: { seq: "3" } });
  const dm = (await request("/api/dms", { auth: true })).value.conversations[0];
  assert.equal(dm.lastSeq, "3");
  assert.equal(dm.readSeq, "3");
  await request("/__fixture/control", {
    method: "POST",
    body: { incomingMessage: { channelId: ids.direct, text: "After reactions" } },
  });
  assert.equal((await request(root, { auth: true })).value.messages.at(-1).seq, "4");
});

test("pins are channel-wide, replayable, idempotent and separate from history pagination", async (t) => {
  const { request, fixture } = await setup(t);
  const root = `/api/chat/channels/${ids.demo}/messages`;
  const target = (await request(root, { auth: true })).value.messages[0];
  const session = await request("/api/chat/session", { auth: true, method: "POST", body: { name: "Fixture Owner" } });
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  await stream.next();
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "pins", kind: "chat", channelId: ids.demo, after: "4" }));
  assert.equal((await stream.next()).event.cursor, "4");
  await stream.next();
  const mutation = {
    auth: true,
    method: "PUT",
    headers: { "x-caper-chat-token": session.value.token },
    body: { active: true },
  };
  const path = `${root}/${target.id}/pin`;
  const pinned = await request(path, mutation);
  assert.equal(pinned.value.seq, "5");
  assert.equal(pinned.value.message.seq, "1");
  assert.equal(pinned.value.message.pinSeq, "5");
  assert.equal(pinned.value.message.pin.author.id, ids.owner);
  assert.deepEqual((await stream.next()).event, pinned.value);
  assert.deepEqual((await request(path, mutation)).value, pinned.value, "no-op does not allocate another sequence");
  for (let index = 0; index < 51; index++)
    await request("/__fixture/control", {
      method: "POST",
      body: { incomingMessage: { channelId: ids.demo, text: `after old pin ${index}` } },
    });
  const history = (await request(root, { auth: true })).value;
  assert.equal(history.messages.length, 50);
  assert.ok(history.messages.every((message) => message.id !== target.id));
  assert.equal(history.pinnedMessages[0].id, target.id);
  assert.equal(history.cursor, "56");
  await request("/__fixture/control", {
    method: "POST",
    body: { incomingPin: { channelId: ids.demo, messageId: target.id, active: false } },
  });
  const unpinned = (await request(root, { auth: true })).value;
  assert.equal(unpinned.cursor, "57");
  assert.deepEqual(unpinned.pinnedMessages, []);
  const older = (await request(`${root}?before=${history.messages[0].seq}`, { auth: true })).value;
  assert.equal(older.messages[0].pin, null);
  assert.equal(older.messages[0].pinSeq, "57");
  const previewRoot = `/api/chat/channels/${ids.general}/messages`;
  const previewTarget = (await request(previewRoot, { auth: true })).value.messages[0].id;
  await request(`/api/spaces/${ids.space}/channels/${ids.general}/membership`, { auth: true, method: "DELETE" });
  assert.equal(
    (await request(`${previewRoot}/${previewTarget}/pin`, mutation)).response.status,
    404,
    "previews cannot mutate pins",
  );
});

test("message IDs match the API shape native clients accept for reaction paths", async (t) => {
  const { request } = await setup(t);
  const root = `/api/chat/channels/${ids.general}/messages`;
  const session = await request("/api/chat/session", { auth: true, method: "POST", body: { name: "Fixture Owner" } });
  await request(root, {
    auth: true,
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { clientMessageId: randomUUID(), text: "TEST FIXTURE — sent" },
  });
  await request("/__fixture/control", {
    method: "POST",
    body: { incomingMessage: { channelId: ids.general, text: "live" } },
  });
  const { messages } = (await request(root, { auth: true })).value;
  assert.equal(messages.length, 6);
  for (const message of messages)
    assert.match(message.id, /^[A-Za-z0-9]{15}$/, `${message.id} must be a 15-character alphanumeric ID`);
  const target = messages[0].id;
  const reacted = await request(`${root}/${target}/reactions`, {
    auth: true,
    method: "PUT",
    headers: { "x-caper-chat-token": session.value.token },
    body: { emoji: "🚀", active: true },
  });
  assert.deepEqual(reacted.value.reactions, [{ emoji: "🚀", authorIds: [ids.owner] }]);
  assert.deepEqual(
    (await request(root, { auth: true })).value.messages.find((message) => message.id === target).reactions,
    [{ emoji: "🚀", authorIds: [ids.owner] }],
  );
});

test("sent messages resolve @mentions like the API: any account, specials outside DMs", async (t) => {
  const { request } = await setup(t);
  const session = await request("/api/chat/session", { auth: true, method: "POST", body: { name: "Fixture Owner" } });
  const send = (channel, text) =>
    request(`/api/chat/channels/${channel}/messages`, {
      auth: true,
      method: "POST",
      headers: { "x-caper-chat-token": session.value.token },
      body: { clientMessageId: randomUUID(), text },
    });
  const text = "@Maya @alex @sam @nobody @everyone @here bob@maya.com @maya";
  const tagged = [
    { type: "user", id: ids.member, username: "maya" },
    { type: "user", id: ids.other, username: "alex" },
    { type: "user", id: ids.invitee, username: "sam" },
  ];
  assert.deepEqual((await send(ids.general, text)).value.content.mentions, [
    ...tagged,
    { type: "everyone" },
    { type: "here" },
  ]);
  // Alex lacks the private grant and Sam is not in the space; both are still tagged.
  assert.deepEqual((await send(ids.private, text)).value.content.mentions, [
    ...tagged,
    { type: "everyone" },
    { type: "here" },
  ]);
  assert.equal((await send(ids.general, "no one")).value.content.mentions, undefined);
  await request("/api/dms", { auth: true, method: "POST", body: { username: "fixture_alex" } });
  assert.deepEqual(
    (await send(ids.direct, text)).value.content.mentions,
    tagged,
    "DMs tag anyone but have no specials",
  );
  const seeded = (await request(`/api/chat/channels/${ids.general}/messages`, { auth: true })).value.messages[3];
  assert.deepEqual(seeded.content.mentions, [{ type: "user", id: ids.owner, username: "fixture_owner" }]);
});

test("people lists everyone sharing a space or DM, never yourself", async (t) => {
  const { request } = await setup(t);
  assert.equal((await request("/api/people")).response.status, 401);
  const owner = await request("/api/people", { auth: true });
  assert.deepEqual(
    owner.value.people.map((person) => person.username),
    ["alex", "maya"],
  );
  assert.deepEqual(Object.keys(owner.value.people[0]).sort(), ["avatarId", "displayName", "id", "username"]);
  const member = await request("/api/people", { auth: "fixture-member-token" });
  assert.deepEqual(
    member.value.people.map((person) => person.username),
    ["alex", "fixture_owner"],
  );
});

test("who-reacted lists people in reaction order for readers only", async (t) => {
  const { request } = await setup(t);
  const target = `${ids.general}m01`;
  const path = `/api/chat/channels/${ids.general}/messages/${target}/reactions`;
  for (const userId of [ids.other, ids.member]) {
    const seeded = await request("/__fixture/control", {
      method: "POST",
      body: { incomingReaction: { channelId: ids.general, messageId: target, emoji: "👍", userId } },
    });
    assert.equal(seeded.response.status, 200);
  }
  const list = await request(path, { auth: true });
  assert.equal(list.response.status, 200);
  assert.equal(list.value.messageId, target);
  assert.match(list.value.reactionSeq, /^[1-9]\d*$/);
  assert.deepEqual(list.value.reactions, [
    {
      emoji: "👍",
      authors: [
        { id: ids.other, username: "alex", displayName: "Alex", avatarId: 799 },
        { id: ids.member, username: "maya", displayName: "Maya", avatarId: 31 },
      ],
    },
  ]);
  assert.equal((await request(path)).response.status, 404, "signed-out readers cannot see who reacted");
  assert.equal(
    (await request(`/api/chat/channels/${ids.general}/messages/absent/reactions`, { auth: true })).response.status,
    404,
  );
  const unknown = await request("/__fixture/control", {
    method: "POST",
    body: { incomingReaction: { channelId: ids.general, messageId: target, emoji: "👍", userId: "nobody" } },
  });
  assert.equal(unknown.response.status, 400);
});

test("threads isolate replies, broadcast one shared message and preserve retry identity", async (t) => {
  const { request } = await setup(t);
  const path = `/api/chat/channels/${ids.general}/messages`;
  const session = (await request("/api/chat/session", { method: "POST", auth: true, body: { name: "ignored" } })).value;
  const initial = (await request(path, { auth: true })).value;
  const root = initial.messages[1];
  const body = {
    clientMessageId: randomUUID(),
    text: "TEST FIXTURE thread only",
    threadRootId: root.id,
    broadcast: false,
  };
  const send = (body) =>
    request(path, { method: "POST", auth: true, headers: { "x-caper-chat-token": session.token }, body });
  const first = (await send(body)).value;
  assert.equal(first.threadRootId, root.id);
  assert.equal(first.thread.replyCount, 1);
  assert.deepEqual((await send(body)).value, first);
  assert.equal((await send({ ...body, broadcast: true })).response.status, 409);
  const broadcast = (
    await send({ ...body, clientMessageId: randomUUID(), text: "TEST FIXTURE shared broadcast", broadcast: true })
  ).value;
  const channel = (await request(path, { auth: true })).value;
  assert.equal(channel.messages.length, initial.messages.length + 1);
  assert.ok(!channel.messages.some((message) => message.id === first.id));
  assert.equal(channel.messages.filter((message) => message.id === broadcast.id).length, 1);
  const threadPath = `${path}/${root.id}/thread`;
  const thread = (await request(threadPath, { auth: true })).value;
  assert.deepEqual(
    thread.messages.map((message) => message.id),
    [first.id, broadcast.id],
  );
  assert.equal(thread.root.thread.replyCount, 2);
  assert.deepEqual(
    thread.root.thread.participants.map((person) => person.id),
    [ids.owner],
  );
  const older = (await request(`${threadPath}?before=${broadcast.seq}`, { auth: true })).value;
  assert.deepEqual(
    older.messages.map((message) => message.id),
    [first.id],
  );
  assert.equal((await request(threadPath)).response.status, 401);
  assert.equal((await request(`${path}/${first.id}/thread`, { auth: true })).response.status, 404);
  assert.equal((await send({ ...body, clientMessageId: randomUUID(), threadRootId: first.id })).response.status, 404);
  const reactions = (
    await request(`${path}/${broadcast.id}/reactions`, {
      method: "PUT",
      auth: true,
      headers: { "x-caper-chat-token": session.token },
      body: { emoji: "👍", active: true },
    })
  ).value;
  assert.equal(reactions.messageId, broadcast.id);
  assert.deepEqual(
    (await request(threadPath, { auth: true })).value.messages.at(-1).reactions,
    (await request(path, { auth: true })).value.messages.at(-1).reactions,
  );
});

test("forward failures retry loads and a destination-scoped uncertain send without duplicating partial success", async (t) => {
  const { request, fixture } = await setup(t);
  const control = (body) => request("/__fixture/control", { method: "POST", body });
  const destinations = "/api/chat/forward-destinations";
  assert.equal((await request(destinations)).response.status, 401);
  await control({ failure: { path: destinations, method: "GET", status: 503, persistent: true } });
  for (let attempt = 0; attempt < 2; attempt++)
    assert.equal((await request(destinations, { auth: true })).response.status, 503);
  await control({ clearFailures: true });
  assert.deepEqual((await request(destinations, { auth: true })).value.destinations, [
    { id: ids.general, name: "general", spaceId: ids.space, spaceName: "Fixture Studio", direct: false },
    { id: ids.design, name: "design", spaceId: ids.space, spaceName: "Fixture Studio", direct: false },
    { id: ids.private, name: "planning", spaceId: ids.space, spaceName: "Fixture Studio", direct: false },
  ]);

  const session = (await request("/api/chat/session", { method: "POST", auth: true })).value;
  const mutation = { auth: true, method: "POST", headers: { "x-caper-chat-token": session.token } };
  const send = (channelId, body, options = mutation) =>
    request(`/api/chat/channels/${channelId}/forwards`, { ...options, body });
  const body = {
    sourceChannelId: ids.private,
    sourceMessageId: `${ids.private}m01`,
    clientMessageId: "AB12CD34-EF56-4789-8ABC-DEF012345678",
    text: "Please read this original",
  };
  const uncertain = {
    path: `/api/chat/channels/${ids.design}/forwards`,
    method: "POST",
    status: 503,
    error: "TEST FIXTURE: forward acknowledgement lost.",
    afterPersist: true,
  };
  assert.equal((await control({ failure: { ...uncertain, persistent: true } })).response.status, 400);
  await control({ failure: uncertain });
  // Rejected requests must not consume the post-persistence failure.
  assert.equal((await send(ids.design, { ...body, sourceMessageId: "missing" })).response.status, 404);
  assert.equal((await send(ids.design, body, { auth: true, method: "POST" })).response.status, 401);
  for (const invalid of [
    { clientMessageId: "not-a-uuid" },
    { text: null },
    { text: "   " },
    { text: "\u0000" },
    { text: "a".repeat(4001) },
    { forward: { message: { content: { text: "forged source" } } } },
  ])
    assert.equal((await send(ids.design, { ...body, ...invalid })).response.status, 400);

  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`, {
    headers: { authorization: "Bearer fixture-owner-token" },
  });
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, "hello");
  for (const channelId of [ids.general, ids.design]) {
    stream.ws.send(JSON.stringify({ type: "subscribe", id: channelId, kind: "chat", channelId, after: "4" }));
    assert.deepEqual((await stream.next()).event, { type: "ready", cursor: "4" });
    assert.equal((await stream.next()).type, "subscribed");
  }
  const first = await send(ids.general, body);
  assert.equal(first.response.status, 200, "another destination cannot consume the failure");
  assert.equal((await stream.next()).event.message.id, first.value.id);
  const second = await send(ids.design, body);
  assert.equal(second.response.status, 503);
  assert.deepEqual(second.value, { error: uncertain.error });
  const persisted = (await stream.next()).event;
  assert.equal(persisted.type, "message.created");
  assert.equal(persisted.channelId, ids.design);
  assert.equal(persisted.seq, "5");
  assert.equal(persisted.message.forward.seq, "4");
  assert.equal(persisted.message.forward.message.id, `${ids.private}m01`);
  assert.equal(persisted.message.content.text, "Please read this original");
  assert.equal(persisted.message.clientMessageId, body.clientMessageId.toLowerCase());
  assert.notEqual(persisted.message.id, first.value.id, "retry keys are destination-scoped");
  for (const clientMessageId of [body.clientMessageId, body.clientMessageId.toLowerCase()]) {
    const retry = await send(ids.design, { ...body, clientMessageId });
    assert.equal(retry.response.status, 200);
    assert.deepEqual(retry.value, persisted.message);
  }
  assert.equal((await send(ids.general, body)).value.id, first.value.id);
  const canonicalRetry = await send(ids.design, {
    ...body,
    sourceChannelId: ids.general,
    sourceMessageId: first.value.id,
  });
  assert.equal(canonicalRetry.response.status, 200, "hash the original, not the selected wrapper");
  assert.deepEqual(canonicalRetry.value, persisted.message);
  for (const changed of [{ text: "Different note" }, { sourceMessageId: `${ids.private}m02` }])
    assert.equal((await send(ids.design, { ...body, ...changed })).response.status, 409);
  const anotherSession = (await request("/api/chat/session", { method: "POST", auth: true })).value;
  assert.equal(
    (await send(ids.design, body, { ...mutation, headers: { "x-caper-chat-token": anotherSession.token } })).response
      .status,
    409,
  );
  for (const channelId of [ids.general, ids.design]) {
    const history = (await request(`/api/chat/channels/${channelId}/messages`, { auth: true })).value;
    assert.equal(history.cursor, "5");
    assert.equal(history.messages.length, 5);
    assert.equal(history.messages.filter((message) => message.forward).length, 1);
  }
  stream.ws.send(JSON.stringify({ type: "heartbeat" }));
  assert.equal((await stream.next()).type, "heartbeat", "retries and conflicts publish no duplicate events");

  const threadPath = `/api/chat/channels/${ids.design}/forwards/${persisted.message.id}/thread`;
  await control({ failure: { path: threadPath, method: "GET", status: 503 } });
  assert.equal((await request(threadPath, { auth: true })).response.status, 503);
  const loaded = await request(threadPath, { auth: true });
  assert.equal(loaded.response.status, 200);
  assert.equal(loaded.value.root.id, `${ids.private}m01`);
  assert.equal(loaded.value.source.spaceId, ids.space);
  assert.equal(loaded.value.source.message.id, `${ids.private}m01`);
  assert.equal(loaded.value.cursor, "4");
  assert.deepEqual(loaded.value.messages, []);
  assert.equal(loaded.value.hasMore, false);

  // Explicit clearing and reset clear uncertain failures too, not just GETs.
  await control({ failure: uncertain });
  await control({ clearFailures: true });
  assert.equal((await send(ids.design, { ...body, clientMessageId: randomUUID() })).response.status, 200);
  await control({ failure: uncertain });
  await control({ reset: true });
  const resetSession = (await request("/api/chat/session", { method: "POST", auth: true })).value;
  assert.equal(
    (await send(ids.design, body, { ...mutation, headers: { "x-caper-chat-token": resetSession.token } })).response
      .status,
    200,
  );
});

test("forwards share a live canonical original without granting source access and revoke deleted grants", async (t) => {
  const { request, fixture } = await setup(t);
  const ownerSession = (await request("/api/chat/session", { auth: true, method: "POST" })).value;
  const otherSession = (await request("/api/chat/session", { auth: "fixture-other-token", method: "POST" })).value;
  const owner = { auth: true, headers: { "x-caper-chat-token": ownerSession.token } };
  const other = { auth: "fixture-other-token", headers: { "x-caper-chat-token": otherSession.token } };
  const sourcePath = `/api/chat/channels/${ids.private}/messages`;
  const originalId = `${ids.private}m01`;
  const input = {
    sourceChannelId: ids.private,
    sourceMessageId: originalId,
    clientMessageId: randomUUID(),
    text: "First note @maya @feedback",
  };
  const shared = await request(`/api/chat/channels/${ids.general}/forwards`, { ...owner, method: "POST", body: input });
  assert.equal(shared.response.status, 200);
  assert.deepEqual(shared.value.content.mentions, [{ type: "user", id: ids.member, username: "maya" }]);
  assert.equal((await request(sourcePath, other)).response.status, 404);
  assert.equal(
    (await request(`/api/chat/channels/${ids.design}/forwards`, { ...other, method: "POST", body: input })).response
      .status,
    404,
    "a guessed source ID cannot bypass source access",
  );
  const againInput = {
    sourceChannelId: ids.general,
    sourceMessageId: shared.value.id,
    clientMessageId: randomUUID(),
    text: "Second note",
  };
  const again = await request(`/api/chat/channels/${ids.design}/forwards`, {
    ...other,
    method: "POST",
    body: againInput,
  });
  assert.equal(again.response.status, 200);
  assert.equal(again.value.forward.message.id, originalId);
  assert.equal(again.value.forward.message.channelId, ids.private);
  assert.equal(again.value.forward.message.content.text, "TEST FIXTURE — local sample data, not a live conversation.");
  assert.ok(!("forward" in again.value.forward.message));
  const threadPath = `/api/chat/channels/${ids.general}/forwards/${shared.value.id}/thread`;
  const againThreadPath = `/api/chat/channels/${ids.design}/forwards/${again.value.id}/thread`;
  const inaccessible = (await request(threadPath, other)).value;
  assert.equal(inaccessible.root.id, originalId);
  assert.equal(inaccessible.source, null);
  assert.deepEqual(inaccessible.messages, []);
  assert.equal((await request(threadPath, owner)).value.source.message.id, originalId);
  assert.equal((await request(threadPath)).response.status, 404);
  assert.equal(
    (await request(`/api/chat/channels/${ids.general}/forwards/${originalId}/thread`, other)).response.status,
    404,
  );
  assert.equal(
    (
      await request(`/api/chat/channels/${ids.general}/messages/${shared.value.id}`, {
        ...owner,
        method: "PUT",
        body: { text: "Change wrapper note", expectedRevision: 1 },
      })
    ).response.status,
    404,
  );

  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`, {
    headers: { authorization: "Bearer fixture-other-token" },
  });
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  assert.equal((await stream.next()).type, "hello");
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "denied", kind: "chat", channelId: ids.private, after: "4" }));
  assert.equal((await stream.next()).status, 404);
  for (const channelId of [ids.general, ids.design]) {
    stream.ws.send(JSON.stringify({ type: "subscribe", id: channelId, kind: "chat", channelId, after: "5" }));
    assert.deepEqual((await stream.next()).event, { type: "ready", cursor: "5" });
    assert.equal((await stream.next()).type, "subscribed");
  }
  const live = [];
  const projections = async (sourceSeq, destinationSeq) => {
    const events = [];
    for (const wrapper of [shared.value, again.value]) {
      const frame = await stream.next();
      assert.equal(frame.id, wrapper.channelId);
      assert.equal(frame.event.type, "message.forward");
      assert.equal(frame.event.seq, destinationSeq);
      assert.equal(frame.event.message.id, wrapper.id);
      assert.equal(frame.event.message.seq, "5", "creation order does not change with the projection");
      assert.equal(frame.event.message.content.text, wrapper.content.text);
      assert.equal(frame.event.message.forwardSeq, destinationSeq);
      assert.equal(frame.event.message.forward.seq, sourceSeq);
      assert.equal(frame.event.message.forward.message.id, originalId);
      events.push(frame.event);
    }
    live.push(events[0]);
    return events[0].message.forward.message;
  };
  assert.equal(
    (
      await request(`${sourcePath}/${originalId}`, {
        ...owner,
        method: "PUT",
        body: { text: "Edited canonical original", expectedRevision: 1 },
      })
    ).response.status,
    200,
  );
  const edited = await projections("5", "6");
  assert.equal(edited.content.text, "Edited canonical original");
  assert.equal(edited.revision, 2);
  assert.equal(edited.editSeq, "5");
  await request(`${sourcePath}/${originalId}/reactions`, {
    ...owner,
    method: "PUT",
    body: { emoji: "👀", active: true },
  });
  assert.deepEqual((await projections("6", "7")).reactions, [{ emoji: "👀", authorIds: [ids.owner] }]);
  const reply = await request(sourcePath, {
    ...owner,
    method: "POST",
    body: { clientMessageId: randomUUID(), text: "A live reply", threadRootId: originalId },
  });
  assert.equal(reply.response.status, 200);
  const withReply = await projections("7", "8");
  assert.equal(withReply.thread.replyCount, 1);
  assert.equal(withReply.thread.seq, "7");
  assert.deepEqual(
    withReply.thread.participants.map(({ id }) => id),
    [ids.owner],
  );
  await request(`${sourcePath}/${reply.value.id}`, {
    ...owner,
    method: "PUT",
    body: { text: "Edited live reply", expectedRevision: 1 },
  });
  assert.deepEqual((await projections("8", "9")).thread, withReply.thread);
  await request(`${sourcePath}/${reply.value.id}/reactions`, {
    ...owner,
    method: "PUT",
    body: { emoji: "👍", active: true },
  });
  await projections("9", "10");
  const current = (await request(againThreadPath, other)).value;
  assert.equal(current.cursor, "9");
  assert.equal(current.source, null);
  assert.equal(current.root.content.text, "Edited canonical original");
  assert.deepEqual(
    current.messages.map((message) => message.content.text),
    ["Edited live reply"],
  );
  assert.deepEqual(current.messages[0].reactions, [{ emoji: "👍", authorIds: [ids.owner] }]);
  assert.ok(!("threadRootId" in current.messages[0]));
  assert.equal((await request(`${sourcePath}/${originalId}/versions`, other)).response.status, 404);
  // Replay matches the live immutable snapshots, including reply invalidations.
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "replay", kind: "chat", channelId: ids.general, after: "5" }));
  for (const event of live) assert.deepEqual((await stream.next()).event, event);
  assert.deepEqual((await stream.next()).event, { type: "ready", cursor: "10" });
  assert.equal((await stream.next()).type, "subscribed");
  stream.ws.send(JSON.stringify({ type: "unsubscribe", id: "replay" }));
  stream.ws.send(JSON.stringify({ type: "heartbeat" }));
  assert.equal((await stream.next()).type, "heartbeat");

  const deleted = await request(`/api/chat/channels/${ids.general}/messages/${shared.value.id}`, {
    ...owner,
    method: "DELETE",
  });
  assert.equal(deleted.response.status, 200);
  assert.ok(deleted.value.deletedAt);
  assert.equal(deleted.value.content.text, "");
  assert.deepEqual(deleted.value.forward, { message: null, seq: "0" });
  assert.equal(deleted.value.forwardSeq, "11");
  const removal = (await stream.next()).event;
  assert.equal(removal.type, "message.edited");
  assert.equal(removal.message.forwardSeq, removal.message.editSeq);
  const deletedRetry = await request(`/api/chat/channels/${ids.general}/forwards`, {
    ...owner,
    method: "POST",
    body: input,
  });
  assert.equal(deletedRetry.response.status, 200);
  assert.deepEqual(deletedRetry.value, deleted.value);
  assert.equal((await request(threadPath, other)).response.status, 404);
  assert.equal(
    (await request(`/api/chat/channels/${ids.design}/forwards`, { ...other, method: "POST", body: againInput }))
      .response.status,
    404,
    "even a retry must recheck the selected wrapper grant",
  );
  await request(`${sourcePath}/${originalId}`, {
    ...owner,
    method: "PUT",
    body: { text: "Independent forward still lives", expectedRevision: 2 },
  });
  const independent = (await stream.next()).event;
  assert.equal(independent.channelId, ids.design);
  assert.equal(independent.type, "message.forward");
  assert.equal(independent.seq, "11");
  assert.equal(independent.message.forward.message.content.text, "Independent forward still lives");
  stream.ws.send(JSON.stringify({ type: "heartbeat" }));
  assert.equal((await stream.next()).type, "heartbeat", "a deleted wrapper never projects again");
  assert.equal((await request(`${sourcePath}/${originalId}`, { ...owner, method: "DELETE" })).response.status, 200);
  const unavailable = (await stream.next()).event;
  assert.equal(unavailable.channelId, ids.design);
  assert.equal(unavailable.seq, "12");
  assert.deepEqual(unavailable.message.forward, { message: null, seq: "0" });
  assert.deepEqual((await request(againThreadPath, other)).value, {
    root: null,
    messages: [],
    cursor: "0",
    hasMore: false,
    source: null,
  });
  assert.equal(
    (
      await request(`/api/chat/channels/${ids.design}/forwards`, {
        ...other,
        method: "POST",
        body: {
          ...againInput,
          sourceChannelId: ids.design,
          sourceMessageId: again.value.id,
          clientMessageId: randomUUID(),
        },
      })
    ).response.status,
    404,
  );
  // A fresh connection/replay cannot resurrect either deleted excerpt.
  for (const [channelId, head] of [
    [ids.general, "11"],
    [ids.design, "12"],
  ]) {
    const history = (await request(`/api/chat/channels/${channelId}/messages`, other)).value;
    assert.equal(history.cursor, head);
    assert.deepEqual(history.messages.at(-1).forward, { message: null, seq: "0" });
    stream.ws.send(JSON.stringify({ type: "subscribe", id: "tombstones", kind: "chat", channelId, after: "4" }));
    for (let seq = 5; seq <= Number(head); seq++) {
      const event = (await stream.next()).event;
      assert.equal(event.seq, String(seq));
      assert.deepEqual(event.message.forward, { message: null, seq: "0" });
      if (channelId === ids.general) assert.equal(event.message.content.text, "");
    }
    assert.deepEqual((await stream.next()).event, { type: "ready", cursor: head });
    assert.equal((await stream.next()).type, "subscribed");
  }
});

test("forward history paginates only direct replies and shared reply excerpts hide parent navigation", async (t) => {
  const { request } = await setup(t);
  const sessions = await Promise.all(
    [true, "fixture-member-token"].map(async (auth) => ({
      auth,
      headers: { "x-caper-chat-token": (await request("/api/chat/session", { auth, method: "POST" })).value.token },
    })),
  );
  const [owner, member] = sessions;
  const other = { auth: "fixture-other-token" };
  const sourcePath = `/api/chat/channels/${ids.private}/messages`;
  const originalId = `${ids.private}m01`;
  const replies = [];
  // Two authors stay under the production per-account send limit.
  for (let index = 0; index < 52; index++) {
    const sent = await request(sourcePath, {
      ...sessions[index % 2],
      method: "POST",
      body: {
        clientMessageId: randomUUID(),
        text: `Reply ${index}`,
        threadRootId: originalId,
        replyToId: originalId,
        broadcast: index === 0,
      },
    });
    assert.equal(sent.response.status, 200);
    replies.push(sent.value);
  }
  await request(`${sourcePath}/${replies[0].id}/pin`, { ...owner, method: "PUT", body: { active: true } });
  const forward = async (sourceMessageId) => {
    const result = await request(`/api/chat/channels/${ids.general}/forwards`, {
      ...owner,
      method: "POST",
      body: { sourceChannelId: ids.private, sourceMessageId, clientMessageId: randomUUID() },
    });
    assert.equal(result.response.status, 200);
    assert.equal(result.value.content.text, "", "the note is optional");
    return `/api/chat/channels/${ids.general}/forwards/${result.value.id}/thread`;
  };
  const threadPath = await forward(originalId);
  const latest = (await request(threadPath, other)).value;
  assert.equal(latest.root.thread.replyCount, 52);
  assert.equal(latest.root.thread.seq, "56");
  assert.equal(latest.cursor, "57");
  assert.equal(latest.hasMore, true);
  assert.equal(latest.source, null);
  assert.deepEqual(
    latest.messages.map(({ content }) => content.text),
    Array.from({ length: 50 }, (_, i) => `Reply ${i + 2}`),
  );
  const earlier = (await request(`${threadPath}?before=7`, other)).value;
  assert.equal(earlier.hasMore, false);
  assert.equal(earlier.cursor, "57");
  assert.equal(earlier.root.id, originalId);
  assert.deepEqual(
    earlier.messages.map(({ content }) => content.text),
    ["Reply 0", "Reply 1"],
  );
  for (const message of [...earlier.messages, ...latest.messages])
    for (const key of ["threadRootId", "thread", "broadcast", "pin", "pinSeq", "replyTo", "forward", "forwardSeq"])
      assert.ok(!(key in message), `shared replies omit ${key}`);
  const empty = (await request(`${threadPath}?before=0`, other)).value;
  assert.deepEqual(empty.messages, []);
  assert.equal(empty.root.id, originalId);
  assert.equal(empty.cursor, "57");
  for (const before of ["invalid", "-1", "9223372036854775808"])
    assert.equal((await request(`${threadPath}?before=${before}`, other)).response.status, 400);

  const replyPath = await forward(replies[0].id);
  const excerpt = (await request(replyPath, other)).value;
  assert.equal(excerpt.root.id, replies[0].id);
  assert.equal(excerpt.root.content.text, "Reply 0");
  assert.deepEqual(excerpt.messages, [], "sharing a reply does not share its parent or siblings");
  assert.equal(excerpt.source, null);
  for (const key of ["threadRootId", "thread", "broadcast", "pin", "pinSeq", "replyTo"])
    assert.ok(!(key in excerpt.root));
  const navigable = (await request(replyPath, member)).value.source;
  assert.equal(navigable.spaceId, ids.space);
  assert.equal(navigable.message.threadRootId, originalId);
  assert.equal(navigable.message.replyTo.id, originalId);
  assert.equal(navigable.message.broadcast, true);
  assert.ok(navigable.message.pin);
  assert.equal((await request(`${sourcePath}/${originalId}/thread`, other)).response.status, 404);

  await request(`/api/spaces/${ids.space}/channels/${ids.private}/membership`, { ...member, method: "DELETE" });
  assert.equal((await request(sourcePath, member)).response.status, 404);
  assert.equal((await request(replyPath, member)).value.source, null, "source navigation is checked on each read");
  assert.equal((await request(replyPath, member)).value.root.id, replies[0].id);
  await request(`/api/spaces/${ids.space}/channels/${ids.general}/membership`, { ...member, method: "DELETE" });
  assert.ok(
    !(await request("/api/chat/forward-destinations", member)).value.destinations.some(({ id }) => id === ids.general),
  );
  assert.equal((await request(threadPath, member)).response.status, 200, "public preview remains readable");
  assert.equal(
    (
      await request(`/api/chat/channels/${ids.general}/forwards`, {
        ...member,
        method: "POST",
        body: { sourceChannelId: ids.general, sourceMessageId: `${ids.general}m01`, clientMessageId: randomUUID() },
      })
    ).response.status,
    404,
    "reading is not permission to send into an unjoined destination",
  );

  await request(`${sourcePath}/${replies[0].id}`, { ...owner, method: "DELETE" });
  const tombstone = (await request(`${threadPath}?before=7`, other)).value.messages[0];
  assert.equal(tombstone.id, replies[0].id);
  assert.ok(tombstone.deletedAt);
  assert.equal(tombstone.content.text, "");
  assert.deepEqual((await request(replyPath, other)).value, {
    root: null,
    messages: [],
    cursor: "0",
    hasMore: false,
    source: null,
  });
  await request(`/api/spaces/${ids.space}/members/${ids.other}`, { ...owner, method: "DELETE" });
  assert.equal(
    (await request(threadPath, other)).response.status,
    404,
    "destination membership still gates the shared history",
  );
});

test("editing retains versions, author ownership, thread identity and original send retries", async (t) => {
  const { request } = await setup(t);
  const path = `/api/chat/channels/${ids.general}/messages`;
  const session = (await request("/api/chat/session", { method: "POST", auth: true, body: { name: "ignored" } })).value;
  const mutation = { method: "PUT", auth: true, headers: { "x-caper-chat-token": session.token } };
  const send = (body) => request(path, { ...mutation, method: "POST", body });
  const originalBody = { clientMessageId: randomUUID(), text: "TEST FIXTURE Meet Friday 🙂" };
  const original = (await send(originalBody)).value;
  const target = `${path}/${original.id}`;
  const hidden = (await send({ clientMessageId: randomUUID(), text: "Hidden reply", threadRootId: original.id })).value;
  const broadcast = (
    await send({ clientMessageId: randomUUID(), text: "Broadcast reply", threadRootId: original.id, broadcast: true })
  ).value;
  const changed = (
    await request(target, { ...mutation, body: { text: "TEST FIXTURE Meet Saturday 🚀", expectedRevision: 1 } })
  ).value;
  assert.equal(changed.revision, 2);
  assert.equal(changed.seq, original.seq);
  assert.equal(changed.createdAt, original.createdAt);
  assert.equal(changed.thread.replyCount, 2);
  assert.equal(changed.thread.seq, broadcast.seq);
  assert.equal(changed.editSeq, "8");
  assert.deepEqual((await send(originalBody)).value, changed);
  assert.deepEqual(
    (await request(target, { ...mutation, body: { text: changed.content.text, expectedRevision: 1 } })).value,
    changed,
  );
  assert.equal(
    (await request(target, { ...mutation, body: { text: "Stale draft", expectedRevision: 1 } })).response.status,
    409,
  );
  for (const reply of [hidden, broadcast])
    await request(`${path}/${reply.id}`, {
      ...mutation,
      body: { text: `${reply.content.text} corrected`, expectedRevision: 1 },
    });
  const history = (await request(path, { auth: true })).value;
  const thread = (await request(`${target}/thread`, { auth: true })).value;
  assert.equal(history.cursor, "10");
  assert.ok(!history.messages.some((row) => row.id === hidden.id));
  assert.equal(history.messages.find((row) => row.id === broadcast.id).content.text, "Broadcast reply corrected");
  assert.equal(thread.messages.find((row) => row.id === broadcast.id).content.text, "Broadcast reply corrected");
  assert.equal(thread.root.content.text, changed.content.text);
  const versions = (await request(`${target}/versions`, { auth: true })).value;
  assert.deepEqual(
    versions.versions.map((version) => [version.revision, version.content.text]),
    [
      [2, changed.content.text],
      [1, original.content.text],
    ],
  );
  assert.equal((await request(`${target}/versions`)).response.status, 404);
  const peer = history.messages.find((row) => row.author.id === ids.member);
  assert.equal(
    (await request(`${path}/${peer.id}`, { ...mutation, body: { text: "Owner override", expectedRevision: 1 } }))
      .response.status,
    403,
  );
  await request(`/api/spaces/${ids.space}/channels/${ids.general}/membership`, { auth: true, method: "DELETE" });
  assert.equal((await request(`${target}/versions`, { auth: true })).response.status, 200);
  assert.equal(
    (await request(target, { ...mutation, body: { text: "Preview edit", expectedRevision: 2 } })).response.status,
    404,
  );
});

test("deleting leaves an author-only tombstone delivered as an edit, unpinned and unquotable", async (t) => {
  const { request } = await setup(t);
  const path = `/api/chat/channels/${ids.general}/messages`;
  const session = (await request("/api/chat/session", { method: "POST", auth: true, body: { name: "ignored" } })).value;
  const mutation = { auth: true, headers: { "x-caper-chat-token": session.token } };
  const send = (body) => request(path, { ...mutation, method: "POST", body });
  const original = (await send({ clientMessageId: randomUUID(), text: "TEST FIXTURE Delete me" })).value;
  const target = `${path}/${original.id}`;
  const reply = (await send({ clientMessageId: randomUUID(), text: "Quoting", replyToId: original.id })).value;
  await request(`${target}/pin`, { ...mutation, method: "PUT", body: { active: true } });
  const history = (await request(path, { auth: true })).value;
  const peer = history.messages.find((row) => row.author.id === ids.member);
  assert.equal((await request(`${path}/${peer.id}`, { ...mutation, method: "DELETE" })).response.status, 403);
  const deleted = (await request(target, { ...mutation, method: "DELETE" })).value;
  assert.equal(deleted.content.text, "");
  assert.ok(deleted.deletedAt);
  assert.equal(deleted.revision, 2);
  assert.equal(deleted.pin, null);
  assert.equal((await request(target, { ...mutation, method: "DELETE" })).value.deletedAt, deleted.deletedAt);
  const after = (await request(path, { auth: true })).value;
  assert.equal(after.messages.find((row) => row.id === reply.id).replyTo.deleted, true);
  assert.equal(after.messages.find((row) => row.id === reply.id).replyTo.text, "");
  assert.equal(
    (await request(target, { ...mutation, method: "PUT", body: { text: "Back", expectedRevision: 2 } })).response
      .status,
    404,
  );
  assert.equal((await request(`${target}/versions`, { auth: true })).response.status, 404);
});

test("edit history is paginated and replay keeps the immutable original content", async (t) => {
  const { request, fixture } = await setup(t);
  const path = `/api/chat/channels/${ids.demo}/messages`;
  const original = (await request(path)).value.messages[0];
  for (let revision = 2; revision <= 53; revision++)
    await request("/__fixture/control", {
      method: "POST",
      body: { incomingEdit: { channelId: ids.demo, messageId: original.id, text: `Version ${revision} 🙂` } },
    });
  const history = (await request(`${path}/${original.id}/versions`, { auth: true })).value;
  assert.equal(history.versions.length, 50);
  assert.equal(history.hasMore, true);
  assert.equal(history.versions[0].revision, 53);
  assert.equal(history.versions.at(-1).revision, 4);
  const older = (await request(`${path}/${original.id}/versions?before=4`, { auth: true })).value;
  assert.deepEqual(
    older.versions.map((version) => version.revision),
    [3, 2, 1],
  );
  assert.equal(older.versions.at(-1).content.text, original.content.text);
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`);
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  await stream.next();
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "edits", kind: "chat", channelId: ids.demo, after: "0" }));
  const events = [];
  while (true) {
    const frame = await stream.next();
    if (frame.event?.type === "ready") break;
    if (frame.event) events.push(frame.event);
  }
  assert.equal(events[0].message.content.text, original.content.text);
  assert.deepEqual(
    events.map((event) => event.seq),
    Array.from({ length: 56 }, (_, index) => String(index + 1)),
  );
  assert.equal(events.at(-1).type, "message.edited");
  assert.equal(events.at(-1).message.seq, "1");
  assert.equal(events.at(-1).message.editSeq, "56");
});

test("message requests, blocks and DM privacy follow the API contract", async (t) => {
  const { request } = await setup(t);
  assert.deepEqual(
    (await request("/api/dms", { auth: true })).value,
    { conversations: [] },
    "requests are opt-in fixture state",
  );
  await request("/__fixture/control", { method: "POST", body: { messageRequest: {} } });
  let [incoming] = (await request("/api/dms", { auth: true })).value.conversations;
  assert.deepEqual(
    [incoming.id, incoming.status, incoming.blocked, incoming.peer.username, incoming.lastSeq],
    [ids.request, "incoming", false, "jordan", "1"],
  );
  assert.equal(
    (await request(`/api/chat/channels/${ids.request}/messages`, { auth: true })).value.messages[0].author.id,
    ids.stranger,
  );
  assert.equal((await request(`/api/dms/${ids.request}/decline`, { auth: true, method: "POST" })).response.status, 204);
  assert.deepEqual((await request("/api/dms", { auth: true })).value.conversations, [], "declined requests are hidden");
  assert.ok(
    !(await request("/api/chat/forward-destinations", { auth: true })).value.destinations.some(
      ({ id }) => id === ids.request,
    ),
    "and not offered for forwarding",
  );
  assert.equal((await request(`/api/dms/${ids.request}/accept`, { auth: true, method: "POST" })).response.status, 404);
  const reopened = await request("/api/dms", { auth: true, method: "POST", body: { username: "jordan" } });
  assert.equal(reopened.value.status, "accepted", "messaging the sender accepts the request");

  const alex = await request("/api/dms", { auth: true, method: "POST", body: { username: "fixture_alex" } });
  assert.equal(alex.value.status, "accepted");
  assert.equal((await request(`/api/blocks/${ids.owner}`, { auth: true, method: "PUT" })).response.status, 400);
  assert.equal((await request("/api/blocks/missing", { auth: true, method: "PUT" })).response.status, 404);
  for (let attempt = 0; attempt < 2; attempt++)
    assert.equal((await request(`/api/blocks/${ids.other}`, { auth: true, method: "PUT" })).response.status, 204);
  assert.deepEqual((await request("/api/blocks", { auth: true })).value, {
    blocks: [{ id: ids.other, username: "alex", displayName: "Alex", avatarId: 799 }],
  });
  assert.equal(
    (await request("/api/dms", { auth: true })).value.conversations.find(({ id }) => id === ids.direct).blocked,
    true,
  );
  const session = await request("/api/chat/session", { auth: true, method: "POST", body: { name: "Fixture Owner" } });
  const blockedSend = await request(`/api/chat/channels/${ids.direct}/messages`, {
    auth: true,
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { clientMessageId: randomUUID(), text: "hello" },
  });
  assert.deepEqual([blockedSend.response.status, blockedSend.value.code], [403, "dm_blocked"]);
  const [source] = (await request(`/api/chat/channels/${ids.request}/messages`, { auth: true })).value.messages;
  const blockedForward = await request(`/api/chat/channels/${ids.direct}/forwards`, {
    auth: true,
    method: "POST",
    headers: { "x-caper-chat-token": session.value.token },
    body: { sourceChannelId: ids.request, sourceMessageId: source.id, clientMessageId: randomUUID() },
  });
  assert.deepEqual(
    [blockedForward.response.status, blockedForward.value.code],
    [403, "dm_blocked"],
    "forwards are sends",
  );
  assert.equal((await request(`/api/blocks/${ids.other}`, { auth: true, method: "DELETE" })).response.status, 204);
  assert.deepEqual((await request("/api/blocks", { auth: true })).value, { blocks: [] });

  // Blocking a requester declines the request.
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  await request("/__fixture/control", {
    method: "POST",
    body: { messageRequest: { text: "TEST FIXTURE — second request" } },
  });
  await request(`/api/blocks/${ids.stranger}`, { auth: true, method: "PUT" });
  assert.deepEqual((await request("/api/dms", { auth: true })).value.conversations, []);

  assert.deepEqual((await request("/api/account/privacy", { auth: true })).value, { directMessages: "anyone" });
  assert.equal(
    (await request("/api/account/privacy", { auth: true, method: "PUT", body: { directMessages: "friends" } })).response
      .status,
    400,
  );
  assert.deepEqual(
    (await request("/api/account/privacy", { auth: true, method: "PUT", body: { directMessages: "spaces" } })).value,
    { directMessages: "spaces" },
  );
  assert.deepEqual((await request("/api/account/privacy", { auth: true })).value, { directMessages: "spaces" });
});

test("push config and device registration follow the API contract", async (t) => {
  const { request } = await setup(t);
  const apnsToken = "AB".repeat(32);
  const devices = async () => (await request("/__fixture/push-devices")).value;
  assert.equal((await request("/api/push/config")).response.status, 401);
  assert.deepEqual((await request("/api/push/config", { auth: true })).value, { platforms: [] });
  assert.equal(
    (await request("/__fixture/control", { method: "POST", body: { pushPlatforms: ["webpush"] } })).response.status,
    400,
  );
  const register = (body, auth = true, method = "POST") =>
    request("/api/push/devices", { auth, method, body }).then(({ response, value }) => [response.status, value]);
  assert.deepEqual(await register({ platform: "fcm", token: "fcm-token" }, false), [
    401,
    { error: "Sign in required." },
  ]);
  assert.deepEqual(await register({ platform: "fcm", token: "fcm-token" }), [
    400,
    { error: "push platform unavailable" },
  ]);
  await request("/__fixture/control", { method: "POST", body: { pushPlatforms: ["apns", "fcm"] } });
  assert.deepEqual((await request("/api/push/config", { auth: true })).value, { platforms: ["apns", "fcm"] });
  assert.deepEqual(await register({ platform: "apnsSandbox", token: apnsToken }), [
    400,
    { error: "push platform unavailable" },
  ]);
  for (const body of [
    { platform: "apns", token: "ab".repeat(31) },
    { platform: "apns", token: "zz".repeat(32) },
    { platform: "apns", token: "a".repeat(201) },
    { platform: "fcm", token: "" },
    { platform: "fcm", token: "has space" },
    { platform: "fcm", token: "x".repeat(4097) },
  ])
    assert.deepEqual(await register(body), [400, { error: "invalid push token" }], JSON.stringify(body));
  assert.deepEqual(await register({ platform: "fcm", token: "fcm-token", appId: "bad id" }), [
    400,
    { error: "invalid app id" },
  ]);
  assert.deepEqual(await register({ platform: "apns", token: apnsToken, appId: "chat.caper.ios" }), [204, undefined]);
  assert.deepEqual((await devices()).devices, [
    { platform: "apns", token: apnsToken.toLowerCase(), appId: "chat.caper.ios", userId: ids.owner },
  ]);
  // The member registering the same address takes it over.
  assert.deepEqual(await register({ platform: "apns", token: apnsToken }, "fixture-member-token"), [204, undefined]);
  assert.deepEqual(
    (await devices()).devices.map(({ userId }) => userId),
    [ids.member],
  );
  // The owner's DELETE only revokes the owner's own registration, and is idempotent.
  for (let attempt = 0; attempt < 2; attempt++)
    assert.deepEqual(await register({ platform: "apns", token: apnsToken }, true, "DELETE"), [204, undefined]);
  assert.equal((await devices()).devices.length, 1);
  assert.deepEqual(await register({ platform: "fcm", token: "fcm:token-1" }), [204, undefined]);
  // A new address replaces the session's previous one.
  assert.deepEqual(await register({ platform: "fcm", token: "fcm:token-2" }), [204, undefined]);
  assert.deepEqual(
    (await devices()).devices.map(({ token }) => token),
    [apnsToken.toLowerCase(), "fcm:token-2"],
  );
  // Unregistering still works once the platform is no longer advertised.
  await request("/__fixture/control", { method: "POST", body: { pushPlatforms: [] } });
  assert.deepEqual(await register({ platform: "fcm", token: "fcm:token-2" }, true, "DELETE"), [204, undefined]);
  assert.deepEqual(await register({ platform: "webpush", token: "x" }, true, "DELETE"), [
    400,
    { error: "push platform unavailable" },
  ]);
  const { devices: remaining, requests } = await devices();
  assert.deepEqual(
    remaining.map(({ userId }) => userId),
    [ids.member],
  );
  assert.deepEqual(
    requests.map(({ method, platform }) => `${method} ${platform}`),
    ["POST apns", "POST apns", "DELETE apns", "DELETE apns", "POST fcm", "POST fcm", "DELETE fcm"],
  );
  await request("/__fixture/control", { method: "POST", body: { pushPlatforms: ["fcm"] } });
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  assert.deepEqual((await request("/api/push/config", { auth: true })).value, { platforms: [] });
  assert.deepEqual(await devices(), { devices: [], requests: [] });
});

test("notification settings and overrides validate, persist in memory and reset", async (t) => {
  const { request } = await setup(t);
  const defaults = { level: "all", mobile: "whenInactive", voiceStarted: true, overrides: [] };
  assert.equal((await request("/api/notifications/settings")).response.status, 401);
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value, defaults);
  const put = (path, body, auth = true) =>
    request(path, { auth, method: "PUT", body }).then(({ response, value }) => [response.status, value]);
  for (const [body, error] of [
    [{ level: "mentions", extra: true }, "invalid notification settings"],
    [{ level: null }, "level must be all, mentions or nothing"],
    [{ level: "some" }, "level must be all, mentions or nothing"],
    [{ mobile: "never" }, "mobile must be always or whenInactive"],
    [{ voiceStarted: null }, "invalid notification settings"],
    [{ voiceStarted: "false" }, "invalid notification settings"],
  ])
    assert.deepEqual(await put("/api/notifications/settings", body), [400, { error }], JSON.stringify(body));
  assert.deepEqual(await put("/api/notifications/settings", { level: "mentions" }), [
    200,
    { ...defaults, level: "mentions" },
  ]);
  assert.deepEqual(await put("/api/notifications/settings", { voiceStarted: false }), [
    200,
    { ...defaults, level: "mentions", voiceStarted: false },
  ]);
  assert.deepEqual(
    await put("/api/notifications/settings", { mobile: "always" }),
    [200, { level: "mentions", mobile: "always", voiceStarted: false, overrides: [] }],
    "a missing voiceStarted keeps the saved choice",
  );

  const space = `/api/spaces/${ids.space}/notifications`;
  const channel = `/api/spaces/${ids.space}/channels/${ids.design}/notifications`;
  const direct = `/api/dms/${ids.direct}/notifications`;
  assert.deepEqual(await put(space, {}, false), [401, { error: "Sign in required." }]);
  const inHalfAnHour = new Date(Date.now() + 30 * 60 * 1000);
  const muted = new Date(Math.floor(inHalfAnHour.getTime() / 1000) * 1000).toISOString().replace(".000Z", "Z");
  for (const [body, error] of [
    [{ level: "loud" }, "level must be all, mentions, nothing or null"],
    [{ muted: true }, "invalid notification settings"],
    [[], "invalid notification settings"],
    [{ voiceStarted: "false" }, "invalid notification settings"],
    [{ mutedUntil: "tomorrow" }, "mutedUntil must be forever or a time within the next year"],
    [{ mutedUntil: "2020-01-01T00:00:00Z" }, "mutedUntil must be forever or a time within the next year"],
    [
      { mutedUntil: new Date(Date.now() + 400 * 24 * 60 * 60 * 1000).toISOString() },
      "mutedUntil must be forever or a time within the next year",
    ],
  ])
    assert.deepEqual(await put(space, body), [400, { error }], JSON.stringify(body));
  assert.deepEqual(await put(space, { level: "mentions" }), [
    200,
    { spaceId: ids.space, level: "mentions", mutedUntil: null, voiceStarted: null },
  ]);
  assert.deepEqual(
    await put(space, { mutedUntil: inHalfAnHour.toISOString() }),
    [200, { spaceId: ids.space, level: "mentions", mutedUntil: muted, voiceStarted: null }],
    "a missing key leaves that field unchanged",
  );
  // Like push::settings::update_space: only spaces choose whether voice starting notifies.
  assert.deepEqual(await put(space, { voiceStarted: false }), [
    200,
    { spaceId: ids.space, level: "mentions", mutedUntil: muted, voiceStarted: false },
  ]);
  assert.deepEqual(await put(channel, { voiceStarted: false }), [400, { error: "invalid notification settings" }]);
  assert.deepEqual(await put(direct, { voiceStarted: true }), [400, { error: "invalid notification settings" }]);
  assert.deepEqual(await put(channel, { mutedUntil: "forever" }), [
    200,
    { spaceId: ids.space, channelId: ids.design, level: null, mutedUntil: "forever" },
  ]);
  assert.deepEqual(await put(direct, { level: "mentions" }), [400, { error: "level must be nothing or null" }]);
  assert.deepEqual(await put(direct, {}), [404, { error: "conversation not found" }]);
  await request("/api/dms", { auth: true, method: "POST", body: { username: "fixture_alex" } });
  assert.deepEqual(await put(direct, { level: "nothing", mutedUntil: "forever" }), [
    200,
    { conversationId: ids.direct, level: "nothing", mutedUntil: "forever" },
  ]);
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value, {
    level: "mentions",
    mobile: "always",
    voiceStarted: false,
    overrides: [
      { spaceId: ids.space, level: "mentions", mutedUntil: muted, voiceStarted: false },
      { spaceId: ids.space, channelId: ids.design, level: null, mutedUntil: "forever" },
      { conversationId: ids.direct, level: "nothing", mutedUntil: "forever" },
    ],
  });
  assert.deepEqual(
    await put(direct, { level: null, mutedUntil: null }),
    [200, { conversationId: ids.direct, level: null, mutedUntil: null }],
    "null resets",
  );

  // No access is 404: Alex has no grant for the private channel, and no space or channel elsewhere.
  const other = "fixture-other-token";
  const privateChannel = `/api/spaces/${ids.space}/channels/${ids.private}/notifications`;
  assert.deepEqual(await put(privateChannel, { level: "all" }, other), [404, { error: "channel not found" }]);
  assert.equal((await put(privateChannel, { level: "all" })).at(0), 200, "the owner can read it");
  assert.deepEqual(await put(`/api/spaces/${ids.demoSpace}/channels/${ids.demo}/notifications`, {}), [
    404,
    { error: "channel not found" },
  ]);
  assert.deepEqual(await put(`/api/spaces/${ids.demoSpace}/channels/${ids.general}/notifications`, {}), [
    404,
    { error: "channel not found" },
  ]);
  assert.deepEqual(await put("/api/spaces/missing/notifications", {}), [404, { error: "space not found" }]);
  assert.deepEqual((await request("/api/notifications/settings", { auth: other })).value, defaults);

  // Expired mutes read as null, and an override with nothing left is omitted.
  vi.useFakeTimers({ toFake: ["Date"] });
  t.onTestFinished(() => vi.useRealTimers());
  vi.setSystemTime(Date.now() + 60 * 60 * 1000);
  assert.deepEqual(
    (await request("/api/notifications/settings", { auth: true })).value.overrides.map(({ mutedUntil }) => mutedUntil),
    [null, "forever", null],
  );
  vi.useRealTimers();
  // A space override with only its voice choice left still lists; reset, it goes.
  await put(space, { level: null, mutedUntil: null });
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value.overrides.at(0), {
    spaceId: ids.space,
    level: null,
    mutedUntil: null,
    voiceStarted: false,
  });
  await put(space, { voiceStarted: null });
  assert.ok(
    !(await request("/api/notifications/settings", { auth: true })).value.overrides.some(
      (entry) => entry.spaceId === ids.space && !entry.channelId,
    ),
  );
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value, defaults);
});

test("attachment uploads go to absolute URLs on the origin the client called", async (t) => {
  const { base, request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true } });
  const reserve = (headers) =>
    request("/api/assets", {
      auth: true,
      method: "POST",
      headers,
      body: { channelId: ids.general, filename: "notes.txt", contentType: "text/plain", byteSize: 5, preview: true },
    });
  // Native clients refuse relative storage URLs, so each must be absolute.
  const direct = (await reserve()).value;
  assert.equal(direct.upload.url, `${base}/api/assets/${direct.id}/fixture-upload`);
  assert.equal(direct.previewUpload.url, `${base}/api/assets/${direct.id}/fixture-preview`);
  assert.equal((await fetch(direct.upload.url, { method: "PUT", body: "hello" })).status, 204);
  const completed = await request(`/api/assets/${direct.id}/complete`, { auth: true, method: "POST" });
  assert.equal(completed.value.size, 5);
  // Behind the web's dev proxy, the browser uploads through the web origin.
  const proxied = (await reserve({ "x-forwarded-host": "localhost:5174", "x-forwarded-proto": "http" })).value;
  assert.equal(proxied.upload.url, `http://localhost:5174/api/assets/${proxied.id}/fixture-upload`);
});

// Real, non-square PNGs with different colours at every position. These are
// prepared upload bytes, not a placeholder or a symmetric one-pixel image.
const preparedImage = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAIAAAASFvFNAAAAFElEQVR4nGP4z8DAAMH//zOA4X8ASskI+C0xXWQAAAAASUVORK5CYII=",
  "base64",
);
const preparedPreview = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGMQMglLdVYEAASlAWaFxYIeAAAAAElFTkSuQmCC",
  "base64",
);
const imageInput = {
  channelId: ids.general,
  filename: "asymmetric prepared.png",
  contentType: "image/png",
  byteSize: preparedImage.length,
  sourceByteSize: preparedImage.length * 3,
  width: 3,
  height: 2,
};

async function readPreparedImage(url, preview = false) {
  const response = await fetch(url);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "image/png");
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.equal(response.headers.get("content-length"), String(bytes.length));
  assert.deepEqual(bytes, preview ? preparedPreview : preparedImage);
  // Decode these known RGB8/filter-0 PNGs with Node's zlib, checking row order
  // and pixels as well as byte equality. This is not a native-renderer test.
  assert.equal(bytes.readUInt32BE(16), preview ? 2 : 3);
  assert.equal(bytes.readUInt32BE(20), preview ? 1 : 2);
  assert.deepEqual([...bytes.subarray(24, 29)], [8, 2, 0, 0, 0]);
  const idat = [];
  for (let offset = 8; offset < bytes.length;) {
    const size = bytes.readUInt32BE(offset);
    if (bytes.toString("ascii", offset + 4, offset + 8) === "IDAT")
      idat.push(bytes.subarray(offset + 8, offset + 8 + size));
    offset += 12 + size;
  }
  assert.deepEqual(
    [...inflateSync(Buffer.concat(idat))],
    preview
      ? [0, 18, 52, 86, 101, 67, 33]
      : [0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 255, 255, 0, 255, 0, 255, 0, 255, 255],
  );
}

test.for([
  { multipart: false, preview: false },
  { multipart: false, preview: true },
  { multipart: true, preview: false },
  { multipart: true, preview: true },
])("prepared images survive upload, send and read (multipart=$multipart, preview=$preview)", async (options, t) => {
  const { fixture, base, request } = await setup(t);
  const path = `/api/chat/channels/${ids.general}/messages`;
  const seeded = (await request(path, { auth: true })).value.messages;
  assert.equal(seeded.length, 4);
  assert.ok(seeded.every((message) => !message.content.attachments));
  assert.equal((await request("/api/assets", { auth: true, method: "POST", body: imageInput })).response.status, 503);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true, multipartPartBytes: 32 } });
  const reserved = await request("/api/assets", {
    auth: true,
    method: "POST",
    body: {
      ...imageInput,
      multipart: options.multipart,
      ...(options.preview ? { preview: { contentType: "image/png", byteSize: preparedPreview.length } } : {}),
    },
  });
  assert.equal(reserved.response.status, 200);
  const asset = reserved.value;
  assert.equal(asset.kind, "image");
  assert.match(asset.id, /^[a-zA-Z0-9]{16}$/);
  const media = `${base}/api/assets/${asset.id}/fixture-file`;
  const complete = () => request(`/api/assets/${asset.id}/complete`, { auth: true, method: "POST" });
  const put = (url, body) => fetch(url, { method: "PUT", body });
  assert.equal((await fetch(media)).status, 404, "an uncompleted image is not public");
  if (options.preview) {
    assert.equal(asset.previewUpload.url, `${base}/api/assets/${asset.id}/fixture-preview`);
    assert.equal((await complete()).response.status, 422);
    assert.equal((await put(asset.previewUpload.url, preparedPreview)).status, 204);
    assert.equal((await fetch(asset.previewUpload.url)).status, 404, "a staged preview is not public either");
  }
  let uploadedURL;
  if (options.multipart) {
    assert.equal(asset.upload, undefined);
    assert.deepEqual(asset.multipart, { partBytes: 32, partCount: 3 });
    const parts = (
      await request(`/api/assets/${asset.id}/parts`, {
        auth: true,
        method: "POST",
        body: { parts: [3, 1, 2, 1] },
      })
    ).value.parts;
    assert.deepEqual(
      parts.map(({ number }) => number),
      [1, 2, 3],
    );
    assert.equal(parts[0].url, `${base}/api/assets/${asset.id}/fixture-part/1`);
    uploadedURL = parts[0].url;
    assert.equal((await put(parts[2].url, preparedImage.subarray(64))).status, 204, "last part can arrive first");
    assert.equal((await put(parts[0].url, preparedImage.subarray(0, 31))).status, 204);
    assert.equal((await complete()).response.status, 422, "wrong-sized parts still fail completion");
    assert.equal((await put(parts[0].url, preparedImage.subarray(0, 32))).status, 204, "a retry replaces the part");
    await request("/__fixture/control", { method: "POST", body: { failPart: 2 } });
    assert.equal((await put(parts[1].url, preparedImage.subarray(32, 64))).status, 500);
    const missing = await complete();
    assert.equal(missing.response.status, 409);
    assert.equal(missing.value.code, "upload_incomplete");
    assert.equal((await put(parts[1].url, preparedImage.subarray(32, 64))).status, 204);
  } else {
    assert.equal(asset.multipart, undefined);
    assert.equal(asset.upload.url, `${base}/api/assets/${asset.id}/fixture-upload`);
    uploadedURL = asset.upload.url;
    assert.equal((await put(uploadedURL, preparedImage.subarray(1))).status, 204);
    assert.equal((await complete()).response.status, 422, "a truncated single PUT still fails completion");
    assert.equal((await put(uploadedURL, preparedImage)).status, 204);
  }
  if (options.preview) {
    assert.equal((await put(asset.previewUpload.url, preparedPreview.subarray(1))).status, 204);
    assert.equal((await complete()).response.status, 422, "a valid original cannot publish a truncated preview");
    assert.equal((await put(asset.previewUpload.url, preparedPreview)).status, 204);
  }
  assert.equal((await fetch(media)).status, 404, "PUT does not publish a prepared image");
  assert.equal(
    (
      await request(`/api/assets/${asset.id}/complete`, {
        auth: "fixture-member-token",
        method: "POST",
      })
    ).response.status,
    404,
  );
  const completed = await complete();
  assert.equal(completed.response.status, 200);
  // The production completion endpoint returns describe(row), without URLs or status.
  const description = {
    id: asset.id,
    name: imageInput.filename,
    kind: "image",
    contentType: "image/png",
    size: preparedImage.length,
    width: 3,
    height: 2,
    ...(options.preview ? { preview: {} } : {}),
  };
  assert.deepEqual(completed.value, description);
  assert.deepEqual((await complete()).value, description, "completion retries retain the same metadata and bytes");
  if (options.multipart) {
    assert.equal(
      (await request(`/__fixture/assets/${asset.id}`)).value.sha256,
      createHash("sha256").update(preparedImage).digest("hex"),
    );
    assert.equal(
      (
        await request(`/api/assets/${asset.id}/parts`, {
          auth: true,
          method: "POST",
          body: { parts: [1] },
        })
      ).response.status,
      404,
    );
  }
  const session = (await request("/api/chat/session", { auth: true, method: "POST", body: { name: "ignored" } })).value;
  const stream = socket(`ws://127.0.0.1:${fixture.gatewayPort}/api/chat/events`, {
    headers: { authorization: "Bearer fixture-owner-token" },
  });
  t.onTestFinished(() => stream.ws.close());
  await stream.opened;
  await stream.next();
  stream.ws.send(JSON.stringify({ type: "subscribe", id: "images", kind: "chat", channelId: ids.general, after: "4" }));
  await stream.next();
  await stream.next();
  const send = {
    auth: true,
    method: "POST",
    headers: { "x-caper-chat-token": session.token },
    body: { clientMessageId: randomUUID(), text: "", attachmentIds: [asset.id] },
  };
  await request("/__fixture/control", { method: "POST", body: { holdSends: true } });
  const sending = request(path, send);
  await vi.waitFor(async () => {
    assert.equal((await request("/__fixture/control", { method: "POST", body: {} })).value.heldSends, 1);
  });
  assert.equal((await request(path, { auth: true })).value.messages.length, 4, "held image sends stay out of history");
  await request("/__fixture/control", { method: "POST", body: { holdSends: false } });
  const sent = await sending;
  assert.equal(sent.response.status, 200);
  const attachment = {
    ...description,
    url: media,
    ...(options.preview ? { previewUrl: asset.previewUpload.url } : {}),
  };
  assert.deepEqual(sent.value.content.attachments, [attachment]);
  assert.deepEqual((await stream.next()).event.message.content.attachments, [attachment]);
  assert.deepEqual((await request(path, send)).value, sent.value, "send retries preserve image delivery URLs");
  assert.deepEqual((await request(`${path}/${sent.value.id}`, { auth: true })).value.content.attachments, [attachment]);
  const history = (await request(path, { auth: true })).value.messages;
  assert.equal(history.length, 5);
  assert.deepEqual(history.at(-1).content.attachments, [attachment]);
  await readPreparedImage(attachment.url);
  if (options.preview) await readPreparedImage(attachment.previewUrl, true);
  else assert.equal((await fetch(`${base}/api/assets/${asset.id}/fixture-preview`)).status, 404);
  assert.equal(
    (await put(uploadedURL, preparedImage.subarray(0, 1))).status,
    404,
    "late PUTs cannot replace published bytes",
  );
  await readPreparedImage(attachment.url);
  // Preserve the fixture's existing completed-asset cancellation behaviour.
  assert.equal((await request(`/api/assets/${asset.id}/cancel`, { auth: true, method: "POST" })).response.status, 204);
  await readPreparedImage(attachment.url);
  await request("/__fixture/control", {
    method: "POST",
    body: {
      failure: { path: new URL(attachment.url).pathname, method: "GET", status: 404, persistent: true },
    },
  });
  assert.equal((await fetch(attachment.url)).status, 404, "existing failure controls can make media unavailable");
  assert.equal((await fetch(attachment.url)).status, 404);
  await request("/__fixture/control", { method: "POST", body: { clearFailures: true } });
  await readPreparedImage(attachment.url);
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  assert.equal((await fetch(attachment.url)).status, 404, "reset revokes old media URLs");
  if (options.preview) assert.equal((await fetch(attachment.previewUrl)).status, 404);
  assert.deepEqual((await request(path, { auth: true })).value.messages, seeded);
});

test.for([false, true])(
  "cancelled images discard pending original/preview media (multipart=%s)",
  async (multipart, t) => {
    const { base, request } = await setup(t);
    await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true, multipartPartBytes: 32 } });
    const asset = (
      await request("/api/assets", {
        auth: true,
        method: "POST",
        body: {
          ...imageInput,
          multipart,
          preview: { contentType: "image/png", byteSize: preparedPreview.length },
        },
      })
    ).value;
    const upload = multipart
      ? (await request(`/api/assets/${asset.id}/parts`, { auth: true, method: "POST", body: { parts: [1] } })).value
          .parts[0].url
      : asset.upload.url;
    const bytes = multipart ? preparedImage.subarray(0, 32) : preparedImage;
    assert.equal((await fetch(upload, { method: "PUT", body: bytes })).status, 204);
    assert.equal((await fetch(asset.previewUpload.url, { method: "PUT", body: preparedPreview })).status, 204);
    assert.equal(
      (await request(`/api/assets/${asset.id}/cancel`, { auth: "fixture-member-token", method: "POST" })).response
        .status,
      404,
    );
    for (let attempt = 0; attempt < 2; attempt++)
      assert.equal(
        (await request(`/api/assets/${asset.id}/cancel`, { auth: true, method: "POST" })).response.status,
        204,
      );
    assert.equal((await request(`/__fixture/assets/${asset.id}`)).value.deleted, true);
    assert.equal(
      (await request(`/api/assets/${asset.id}/complete`, { auth: true, method: "POST" })).response.status,
      404,
    );
    assert.equal((await fetch(upload, { method: "PUT", body: bytes })).status, 404);
    assert.equal((await fetch(asset.previewUpload.url, { method: "PUT", body: preparedPreview })).status, 404);
    assert.equal((await fetch(asset.previewUpload.url)).status, 404);
    assert.equal((await fetch(`${base}/api/assets/${asset.id}/fixture-file`)).status, 404);
  },
);

test("image retention is bounded by validated reservations, request sizes, and a resettable fixture budget", async (t) => {
  const { request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true, multipartPartBytes: 32 } });
  const reserve = (input) => request("/api/assets", { auth: true, method: "POST", body: { ...imageInput, ...input } });
  for (const input of [
    { byteSize: 0 },
    { byteSize: -1 },
    { byteSize: 0.5 },
    { byteSize: "77" },
    { byteSize: Number.MAX_SAFE_INTEGER + 1 },
    { width: 0 },
    { height: 32_769 },
    { durationMs: -1 },
    { durationMs: 86_400_001 },
    { preview: true },
    { preview: { contentType: "image/svg+xml", byteSize: 10 } },
    { preview: { contentType: "image/png", byteSize: 512 * 1024 + 1 } },
  ])
    assert.equal((await reserve(input)).response.status, 400, JSON.stringify(input));
  for (const multipart of [false, true])
    assert.equal((await reserve({ multipart, byteSize: 8 * 1024 * 1024 + 1 })).response.status, 413);
  const single = (await reserve({ preview: { contentType: "image/png", byteSize: preparedPreview.length } })).value;
  assert.equal(
    (await fetch(single.upload.url, { method: "PUT", body: Buffer.alloc(preparedImage.length + 1) })).status,
    413,
  );
  assert.equal(
    (await fetch(single.previewUpload.url, { method: "PUT", body: Buffer.alloc(preparedPreview.length + 1) })).status,
    413,
  );
  const multipart = (await reserve({ multipart: true })).value;
  const parts = (
    await request(`/api/assets/${multipart.id}/parts`, { auth: true, method: "POST", body: { parts: [1, 2] } })
  ).value.parts;
  assert.equal((await fetch(parts[0].url, { method: "PUT", body: preparedImage })).status, 204);
  assert.equal(
    (await fetch(parts[1].url, { method: "PUT", body: Buffer.from([1]) })).status,
    413,
    "parts cannot cumulatively exceed the declared size",
  );
  assert.equal(
    (await request(`/api/assets/${multipart.id}/complete`, { auth: true, method: "POST" })).response.status,
    422,
  );
  await request("/__fixture/control", { method: "POST", body: { reset: true, uploadsEnabled: true } });
  const reservations = [];
  for (let index = 0; index < 8; index++) {
    const result = await reserve({ byteSize: 8 * 1024 * 1024 });
    assert.equal(result.response.status, 200);
    reservations.push(result.value.id);
  }
  assert.equal((await reserve({})).response.status, 413, "even valid images cannot grow retained memory indefinitely");
  await request(`/api/assets/${reservations[0]}/cancel`, { auth: true, method: "POST" });
  assert.equal((await reserve({})).response.status, 200, "cancellation releases its reservation");
  await request("/__fixture/control", { method: "POST", body: { reset: true, uploadsEnabled: true } });
  for (let index = 0; index < 128; index++) assert.equal((await reserve({})).response.status, 200);
  assert.equal((await reserve({})).response.status, 413, "small image reservations also have a count bound");
});

test("image PUTs use the forwarded origin while media, like avatars and voice, uses direct loopback", async (t) => {
  const { base, request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true } });
  const asset = (
    await request("/api/assets", {
      auth: true,
      method: "POST",
      body: imageInput,
      headers: { "x-forwarded-host": "web.fixture.test", "x-forwarded-proto": "https" },
    })
  ).value;
  assert.equal(asset.upload.url, `https://web.fixture.test/api/assets/${asset.id}/fixture-upload`);
  // Model the proxy forwarding the PUT, without contacting a real external host.
  await fetch(base + new URL(asset.upload.url).pathname, { method: "PUT", body: preparedImage });
  await request(`/api/assets/${asset.id}/complete`, { auth: true, method: "POST" });
  const session = (await request("/api/chat/session", { auth: true, method: "POST", body: { name: "ignored" } })).value;
  const message = (
    await request(`/api/chat/channels/${ids.general}/messages`, {
      auth: true,
      method: "POST",
      headers: { "x-caper-chat-token": session.token },
      body: { clientMessageId: randomUUID(), text: "", attachmentIds: [asset.id] },
    })
  ).value;
  assert.equal(message.content.attachments[0].url, `${base}/api/assets/${asset.id}/fixture-file`);
  await readPreparedImage(message.content.attachments[0].url);
});

test("image descriptions omit absent dimensions, retain optional duration, and do not render unsupported MIME types", async (t) => {
  const { base, request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true } });
  for (const contentType of ["image/png", "image/svg+xml"]) {
    const asset = (
      await request("/api/assets", {
        auth: true,
        method: "POST",
        body: {
          ...imageInput,
          contentType,
          width: null,
          height: null,
          durationMs: 0,
        },
      })
    ).value;
    await fetch(asset.upload.url, { method: "PUT", body: preparedImage });
    const completed = await request(`/api/assets/${asset.id}/complete`, { auth: true, method: "POST" });
    assert.deepEqual(completed.value, {
      id: asset.id,
      name: imageInput.filename,
      contentType,
      size: preparedImage.length,
      ...(contentType === "image/png" ? { kind: "image", durationMs: 0 } : { kind: "file", status: "ready" }),
    });
    const media = `${base}/api/assets/${asset.id}/fixture-file`;
    if (contentType === "image/png") await readPreparedImage(media);
    else assert.equal((await fetch(media)).status, 404, "unsupported image types are not exposed as inline media");
  }
});

test("ordinary multipart file hashes and recorded voice media keep their existing fixture contracts", async (t) => {
  const { request } = await setup(t);
  await request("/__fixture/control", { method: "POST", body: { uploadsEnabled: true, multipartPartBytes: 3 } });
  const file = (
    await request("/api/assets", {
      auth: true,
      method: "POST",
      body: {
        channelId: ids.general,
        filename: "notes.txt",
        contentType: "text/plain",
        byteSize: 5,
        multipart: true,
      },
    })
  ).value;
  const parts = (await request(`/api/assets/${file.id}/parts`, { auth: true, method: "POST", body: { parts: [1, 2] } }))
    .value.parts;
  await fetch(parts[1].url, { method: "PUT", body: "lo" });
  await fetch(parts[0].url, { method: "PUT", body: "hel" });
  assert.deepEqual((await request(`/api/assets/${file.id}/complete`, { auth: true, method: "POST" })).value, {
    id: file.id,
    name: "notes.txt",
    contentType: "text/plain",
    size: 5,
    kind: "file",
    status: "ready",
  });
  assert.equal(
    (await request(`/__fixture/assets/${file.id}`)).value.sha256,
    createHash("sha256").update("hello").digest("hex"),
  );
  const voice = (
    await request("/api/assets", {
      auth: true,
      method: "POST",
      body: {
        channelId: ids.general,
        filename: "recorded.ogg",
        contentType: "audio/ogg",
        byteSize: 3,
        voice: true,
        durationMs: 1000,
        waveform: "AQID",
      },
    })
  ).value;
  const bytes = Buffer.from([1, 2, 3]);
  await fetch(voice.upload.url, { method: "PUT", body: bytes });
  const completed = (await request(`/api/assets/${voice.id}/complete`, { auth: true, method: "POST" })).value;
  assert.equal(completed.kind, "audio");
  assert.equal(completed.contentType, "audio/mp4");
  assert.equal(completed.waveform, "AQID");
  assert.equal(completed.durationMs, 1000);
  assert.deepEqual(completed.transcript, { status: "pending" });
  const recorded = await fetch(completed.url);
  assert.equal(
    recorded.headers.get("content-type"),
    "audio/ogg",
    "the fixture still serves recorded bytes without transcoding",
  );
  assert.deepEqual(Buffer.from(await recorded.arrayBuffer()), bytes);
});

test("binds loopback unless a private-LAN host is explicitly configured", async (t) => {
  for (const host of [
    "0.0.0.0",
    "8.8.8.8",
    "172.32.0.1",
    "172.15.0.1",
    "192.168.1.256",
    "010.0.0.1",
    "example.com",
    "",
  ])
    await assert.rejects(startFixture({ port: 0, gatewayPort: 0, host }), /not a loopback or private-LAN/, host);
  for (const host of ["127.0.0.1", "10.0.0.5", "172.16.0.1", "172.31.255.255", "192.168.1.20"])
    assert.equal(fixtureHost(host), host);
  const { fixture } = await setup(t);
  assert.equal(fixture.host, "127.0.0.1");
});

test("formatted bench rows are opt-in and deterministic", async (t) => {
  const { request } = await setup(t);
  const plain = await request(`/api/spaces/${ids.space}`, { auth: true });
  assert.deepEqual(
    plain.value.channels.map((channel) => channel.name),
    ["general", "design", "planning"],
  );
  assert.equal(
    (await request(`/api/chat/channels/${benchFormattingChannel}/messages`, { auth: true })).response.status,
    404,
  );

  const page = async () => {
    const fixture = await startFixture({ port: 0, gatewayPort: 0, formatted: 60 });
    t.onTestFinished(() => fixture.close());
    const base = `http://127.0.0.1:${fixture.port}`;
    const headers = { authorization: "Bearer fixture-owner-token" };
    const space = await (await fetch(`${base}/api/spaces/${ids.space}`, { headers })).json();
    assert.ok(space.channels.some((channel) => channel.id === benchFormattingChannel && !channel.private));
    return (await fetch(`${base}/api/chat/channels/${benchFormattingChannel}/messages`, { headers })).json();
  };
  const first = await page();
  assert.equal(first.messages.length, 50);
  assert.equal(first.hasMore, true);
  for (const { content } of first.messages) {
    assert.ok(content.text.length >= 400, content.text);
    assert.match(content.text, /\*\*[^*]+\*\*/);
    assert.ok((content.text.match(/https?:\/\/|www\./g) ?? []).length >= 4, content.text);
  }
  assert.deepEqual((await page()).messages, first.messages);
});

test("an invite link lands in a public channel, fulfils the invitation, and its space's own channels invite you", async (t) => {
  const { request } = await setup(t);
  const maya = "fixture-member-token";
  await request("/__fixture/control", { method: "POST", body: { spaceInvitation: true } });
  const root = `/api/spaces/${ids.invitedSpace}`;
  const link = await request(`${root}/invite-links`, { method: "POST", auth: maya, body: {} });
  assert.equal(link.value.channel, null);
  const joined = await request(`/api/invites/${link.value.code}/join`, { method: "POST", auth: true });
  // Like the API: no landing channel on the link still opens the oldest public one, joined.
  assert.equal(joined.value.channelId, ids.invitedChannel);
  const spaces = (await request("/api/spaces", { auth: true })).value;
  assert.deepEqual(spaces.invitations, [], "joining by link fulfils the pending invitation");
  const detail = (await request(root, { auth: true })).value;
  assert.equal(detail.channels.find((channel) => channel.id === ids.invitedChannel).joined, true);

  // Maya's new private channel is hers, so she can invite the owner to it.
  const secret = await request(`${root}/channels`, {
    method: "POST",
    auth: maya,
    body: { name: "secret", private: true },
  });
  const invite = await request(`${root}/channels/${secret.value.id}/members`, {
    method: "POST",
    auth: maya,
    body: { username: "fixture_owner" },
  });
  assert.equal(invite.response.status, 201);
  assert.deepEqual(
    (await request(root, { auth: true })).value.channelInvitations.map((item) => item.channel.name),
    ["secret"],
  );
});
