import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { startFixture, fixtureIDs as ids } from "../scripts/native-parity-fixture.mjs";

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

function socket(url) {
  const ws = new WebSocket(url);
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
  assert.equal(before.channelInvitations[0].inviter.username, "fixture_owner");
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 404);
  assert.equal((await request(`${root}/membership`, { ...member, method: "POST" })).response.status, 404);
  assert.equal((await request(`${root}/invitation`, { ...member, method: "POST" })).value.joined, true);
  assert.equal((await request(`/api/chat/channels/${created.value.id}/messages`, member)).response.status, 200);
  assert.equal((await request(`${root}/membership`, { ...member, method: "DELETE" })).response.status, 204);
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
  const defaults = { level: "all", mobile: "whenInactive", overrides: [] };
  assert.equal((await request("/api/notifications/settings")).response.status, 401);
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value, defaults);
  const put = (path, body, auth = true) =>
    request(path, { auth, method: "PUT", body }).then(({ response, value }) => [response.status, value]);
  for (const [body, error] of [
    [{ level: "mentions", extra: true }, "invalid notification settings"],
    [{ level: null }, "level must be all, mentions or nothing"],
    [{ level: "some" }, "level must be all, mentions or nothing"],
    [{ mobile: "never" }, "mobile must be always or whenInactive"],
  ])
    assert.deepEqual(await put("/api/notifications/settings", body), [400, { error }], JSON.stringify(body));
  assert.deepEqual(await put("/api/notifications/settings", { level: "mentions" }), [
    200,
    { ...defaults, level: "mentions" },
  ]);
  assert.deepEqual(await put("/api/notifications/settings", { mobile: "always" }), [
    200,
    { level: "mentions", mobile: "always", overrides: [] },
  ]);

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
    { spaceId: ids.space, level: "mentions", mutedUntil: null },
  ]);
  assert.deepEqual(
    await put(space, { mutedUntil: inHalfAnHour.toISOString() }),
    [200, { spaceId: ids.space, level: "mentions", mutedUntil: muted }],
    "a missing key leaves that field unchanged",
  );
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
    overrides: [
      { spaceId: ids.space, level: "mentions", mutedUntil: muted },
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
  await request("/__fixture/control", { method: "POST", body: { reset: true } });
  assert.deepEqual((await request("/api/notifications/settings", { auth: true })).value, defaults);
});
