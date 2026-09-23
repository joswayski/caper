import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { AppGateway, setAppGatewayForTests, watchPresence } from "../gateway/client.ts";
import { watchPresence as watchMediaPresence } from "../media/presence.ts";
import { callSnapshot } from "../media/events.ts";

class FakeWindow extends EventTarget {
  location = { protocol: "https:", host: "caper.test" };
}

test("media projections reject wrong revisions and track shapes before reconciliation", () => {
  const person = { id: "peer", name: "Peer", muted: true, deafened: false };
  const spectator = { type: "snapshot", revision: 7, participants: [person] };
  assert.deepEqual(callSnapshot(spectator, true).participants, [{ ...person, tracks: [] }]);
  const authenticated = { ...spectator, participants: [{ ...person, tracks: [{ id: "mic", kind: "microphone" }] }] };
  assert.deepEqual(callSnapshot(authenticated, false), authenticated);
  assert.throws(() => callSnapshot(authenticated, true), /Invalid live update/);
  assert.throws(() => callSnapshot(spectator, false), /Invalid live update/);
  for (const revision of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, "7"]) {
    assert.throws(() => callSnapshot({ ...authenticated, revision }, false), /Invalid live update/);
  }
  assert.throws(() => callSnapshot({ ...authenticated, participants: [{ ...person, tracks: [{ id: "camera", kind: "camera" }] }] }, false), /Invalid live update/);
});

class FakeSocket extends EventTarget {
  readonly sent: Array<Record<string, unknown>> = [];
  closed = false;
  readonly url: string;

  constructor(url: string) { super(); this.url = url; }
  send(data: string) { this.sent.push(JSON.parse(data)); }
  close() { this.closed = true; }
  frame(value: unknown) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) })); }
  fail() { this.dispatchEvent(new Event("close")); }
}

function setup(t: TestContext) {
  const originalWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", { configurable: true, value: new FakeWindow() });
  t.after(() => Object.defineProperty(globalThis, "window", { configurable: true, value: originalWindow }));
  let nextId = 0;
  t.mock.method(globalThis.crypto, "randomUUID", () => `00000000-0000-4000-8000-${String(++nextId).padStart(12, "0")}`);
  const sockets: FakeSocket[] = [];
  const gateway = new AppGateway((url) => {
    const socket = new FakeSocket(url);
    sockets.push(socket);
    return socket;
  }, () => 0);
  t.after(() => gateway.destroy());
  const hello = (index: number, serverTime = Date.now()) => sockets[index].frame({
    type: "hello", idleTimeoutSeconds: 600, serverTime,
  });
  const subscribe = (index: number, offset = 0) => sockets[index].sent.filter((frame) => frame.type === "subscribe")[offset];
  return { gateway, sockets, hello, subscribe };
}

test("chat, media, and scoped presence share one credential-free same-origin socket", async (t) => {
  const f = setup(t);
  const chat = f.gateway.subscribe({ kind: "chat", channelId: "general", after: "4" }, { event: () => undefined, cursor: () => "4" });
  const media = f.gateway.subscribe({ kind: "media", token: "media-secret" }, { event: () => undefined });
  const presence = f.gateway.subscribe({ kind: "presence", spaceId: "space", userIds: ["one", "two"] }, { event: () => undefined });
  t.after(() => { chat.unsubscribe(); media.unsubscribe(); presence.unsubscribe(); });

  assert.equal(f.sockets.length, 1);
  assert.equal(f.sockets[0].url, "wss://caper.test/api/chat/events");
  assert.equal(new URL(f.sockets[0].url).search, "", "capabilities never enter the URL");
  f.hello(0);
  const frames = f.sockets[0].sent.filter((frame) => frame.type === "subscribe");
  assert.deepEqual(frames.map((frame) => frame.kind), ["chat", "media", "presence"]);
  assert.equal(frames[1].token, "media-secret");
  assert.deepEqual(frames[2].userIds, ["one", "two"]);
  for (const frame of frames) f.sockets[0].frame({ type: "subscribed", id: frame.id });
  await Promise.all([chat.ready, media.ready, presence.ready]);
});

test("handoff retains the old socket through overlap and promotes only after chat catches up", (t) => {
  const f = setup(t);
  let cursor = "5";
  const delivered: string[] = [];
  const subscription = f.gateway.subscribe({ kind: "chat", channelId: "general", after: cursor }, {
    cursor: () => cursor,
    event: (event) => {
      const value = event as { type: string; seq?: string };
      if (value.type === "message.created" && BigInt(value.seq!) > BigInt(cursor)) {
        cursor = value.seq!;
        delivered.push(cursor);
      }
    },
  });
  t.after(() => subscription.unsubscribe());
  f.hello(0);
  const id = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id, event: { type: "ready", cursor: "5" } });
  f.sockets[0].frame({ type: "subscribed", id });
  f.sockets[0].frame({ type: "migrating" });
  assert.equal(f.sockets.length, 2);
  f.hello(1);
  assert.equal(f.subscribe(1).after, "5");

  f.sockets[0].frame({ type: "event", id, event: { type: "message.created", seq: "6" } });
  f.sockets[1].frame({ type: "event", id, event: { type: "ready", cursor: "5" } });
  f.sockets[1].frame({ type: "subscribed", id });
  assert.equal(f.sockets[0].closed, false, "subscribed is not enough while the old stream is ahead");
  f.sockets[1].frame({ type: "event", id, event: { type: "message.created", seq: "6" } });
  assert.equal(f.sockets[0].closed, true);
  assert.deepEqual(delivered, ["6"], "overlap is delivered once to the logical consumer");
});

test("candidate gaps or failure cannot kill a healthy active stream", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  let cursor = "2";
  const delivered: string[] = [];
  const subscription = f.gateway.subscribe({ kind: "chat", channelId: "general", after: cursor }, {
    cursor: () => cursor,
    event: (event) => {
      const seq = (event as { seq?: string }).seq;
      if (seq && BigInt(seq) > BigInt(cursor)) { cursor = seq; delivered.push(seq); }
    },
  });
  t.after(() => subscription.unsubscribe());
  f.hello(0);
  const id = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id, event: { type: "ready", cursor } });
  f.sockets[0].frame({ type: "subscribed", id });
  f.sockets[0].frame({ type: "migrating" });
  f.hello(1);
  f.sockets[1].frame({ type: "event", id, event: { type: "message.created", seq: "4" } });
  assert.equal(f.sockets[1].closed, true, "a candidate gap rejects only the candidate");
  assert.equal(f.sockets[0].closed, false);
  f.sockets[0].frame({ type: "event", id, event: { type: "message.created", seq: "3" } });
  assert.deepEqual(delivered, ["3"]);
});

test("pending commands retry with identical identity on reconnect, command_pending, and gateway draining", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  const result = f.gateway.command({ method: "media.state", token: "secret", body: { muted: true }, timeoutMs: 5_000 });
  f.hello(0);
  const first = f.sockets[0].sent.find((frame) => frame.type === "command")!;
  f.sockets[0].frame({ type: "result", id: first.id, status: 409, body: { code: "command_pending", error: "pending" } });
  t.mock.timers.tick(100);
  const retry = f.sockets[0].sent.filter((frame) => frame.type === "command").at(-1)!;
  assert.deepEqual(retry, first);

  f.sockets[0].fail();
  t.mock.timers.tick(188);
  f.hello(1);
  const reconnectRetry = f.sockets[1].sent.find((frame) => frame.type === "command")!;
  assert.deepEqual(reconnectRetry, first, "reconnect preserves id and issuedAt for backend deduplication");
  f.sockets[1].frame({ type: "result", id: first.id, status: 503, body: { code: "gateway_draining", error: "draining" } });
  assert.equal(f.sockets.length, 3);
  f.hello(2);
  const drainingRetry = f.sockets[2].sent.find((frame) => frame.type === "command")!;
  assert.deepEqual(drainingRetry, first, "drain retry waits for the replacement and keeps the command identity");
  f.sockets[2].frame({ type: "result", id: first.id, status: 200, body: { ok: true } });
  assert.deepEqual(await result, { ok: true });
});

for (const interruption of ["disconnect", "handoff", "draining", "pending"] as const) {
  test(`typing pulses are not queued or retried after ${interruption}`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const f = setup(t);
    await assert.rejects(f.gateway.command({ method: "typing" }), /reconnecting/);
    assert.equal(f.sockets.length, 0, "typing alone cannot open a connection or queue old activity");
    const subscription = f.gateway.subscribe({ kind: "chat", channelId: "general", after: "0" }, { event: () => undefined });
    void subscription.ready.catch(() => undefined);
    t.after(() => subscription.unsubscribe());
    f.hello(0);
    f.sockets[0].frame({ type: "subscribed", id: subscription.id });
    const pulse = f.gateway.command({ method: "typing", channelId: "general", body: { typing: true }, timeoutMs: 2_000 });
    const lost = assert.rejects(pulse);
    const first = f.sockets[0].sent.find((frame) => frame.type === "command")!;
    assert.equal(first.method, "typing");

    if (interruption === "disconnect") {
      f.sockets[0].fail();
      t.mock.timers.tick(188);
      f.hello(1);
    } else if (interruption === "handoff") {
      f.sockets[0].frame({ type: "migrating" });
      f.hello(1);
      f.sockets[1].frame({ type: "subscribed", id: subscription.id });
      assert.equal(f.sockets[0].closed, true);
    } else {
      f.sockets[0].frame({ type: "result", id: first.id, status: interruption === "draining" ? 503 : 409,
        body: { code: interruption === "draining" ? "gateway_draining" : "command_pending" } });
      t.mock.timers.tick(1_000);
    }
    await lost;
    assert.equal(f.sockets.flatMap((socket) => socket.sent).filter((frame) => frame.type === "command").length, 1);

    const current = f.sockets.at(-1)!;
    const stop = f.gateway.command({ method: "typing", channelId: "general", body: { typing: false } });
    const latest = current.sent.filter((frame) => frame.type === "command").at(-1)!;
    assert.deepEqual(latest.body, { typing: false });
    current.frame({ type: "result", id: latest.id, status: 204, body: null });
    await stop;
  });
}

test("pointer movement reports throttled activity without focus state", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  const subscription = f.gateway.subscribe({ kind: "presence", spaceId: "space", userIds: [] }, { event: () => undefined });
  void subscription.ready.catch(() => undefined);
  t.after(() => subscription.unsubscribe());
  f.hello(0);

  globalThis.window.dispatchEvent(new Event("pointermove"));
  globalThis.window.dispatchEvent(new Event("pointermove"));
  t.mock.timers.tick(999);
  assert.equal(f.sockets[0].sent.filter((frame) => frame.type === "activity").length, 0);
  t.mock.timers.tick(1);
  const activity = f.sockets[0].sent.filter((frame) => frame.type === "activity");
  assert.equal(activity.length, 1);
  assert.equal(typeof activity[0].activityAgeMs, "number");
});

test("a command is stamped once from the first server clock and preserves that stamp on retry", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  const serverTime = Date.now() + 45_000;
  void f.gateway.command({ method: "media.snapshot", timeoutMs: 5_000 }).catch(() => undefined);
  assert.equal(f.sockets[0].sent.some((frame) => frame.type === "command"), false);
  f.hello(0, serverTime);
  const first = f.sockets[0].sent.find((frame) => frame.type === "command")!;
  assert.equal(first.issuedAt, serverTime);

  f.sockets[0].fail();
  t.mock.timers.tick(188);
  f.hello(1, serverTime + 90_000);
  assert.deepEqual(f.sockets[1].sent.find((frame) => frame.type === "command"), first);
});

test("queued frames for an unsubscribed logical ID do not disconnect healthy subscriptions", (t) => {
  const f = setup(t);
  const stale = f.gateway.subscribe({ kind: "chat", channelId: "old" }, { event: () => undefined });
  const received: unknown[] = [];
  const healthy = f.gateway.subscribe({ kind: "chat", channelId: "current" }, { event: (event) => received.push(event) });
  void stale.ready.catch(() => undefined);
  t.after(() => healthy.unsubscribe());
  f.hello(0);
  const staleId = f.subscribe(0, 0).id as string;
  const healthyId = f.subscribe(0, 1).id as string;
  stale.unsubscribe();

  f.sockets[0].frame({ type: "event", id: staleId, event: { type: "typing.updated" } });
  f.sockets[0].frame({ type: "subscribed", id: staleId });
  f.sockets[0].frame({ type: "error", id: staleId, status: 403, error: "expired" });
  f.sockets[0].frame({ type: "event", id: healthyId, event: { type: "typing.updated" } });
  f.sockets[0].frame({ type: "subscribed", id: healthyId });

  assert.equal(f.sockets[0].closed, false);
  assert.deepEqual(received, [{ type: "typing.updated" }]);
});

test("subscription 503 retries the same ID while authorization errors terminate it", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  const live: boolean[] = [];
  const transient = f.gateway.subscribe({ kind: "presence", spaceId: "space", userIds: ["one"] }, {
    event: () => undefined,
    status: (online) => live.push(online),
  });
  f.hello(0);
  const transientId = f.subscribe(0).id;
  f.sockets[0].frame({ type: "error", id: transientId, status: 503, error: "draining" });
  t.mock.timers.tick(187);
  assert.equal(f.sockets[0].sent.filter((frame) => frame.type === "subscribe").length, 1);
  t.mock.timers.tick(1);
  const retried = f.sockets[0].sent.filter((frame) => frame.type === "subscribe");
  assert.equal(retried.length, 2);
  assert.equal(retried[1].id, transientId);
  f.sockets[0].frame({ type: "subscribed", id: transientId });
  await transient.ready;
  assert.deepEqual(live, [false, false, true]);

  f.sockets[0].frame({ type: "migrating" });
  f.hello(1);
  f.sockets[1].frame({ type: "error", id: transientId, status: 503, error: "candidate draining" });
  assert.equal(f.sockets[1].closed, true);
  assert.equal(f.sockets[0].closed, false);
  assert.deepEqual(live, [false, false, true], "candidate failure does not mark the healthy active stream offline");

  const errors: number[] = [];
  const denied = f.gateway.subscribe({ kind: "presence", spaceId: "space", userIds: ["two"] }, {
    event: () => undefined,
    error: (error) => errors.push(error.status),
  });
  const deniedId = f.sockets[0].sent.filter((frame) => frame.type === "subscribe").at(-1)!.id;
  f.sockets[0].frame({ type: "error", id: deniedId, status: 403, error: "forbidden" });
  await assert.rejects(denied.ready, (error: unknown) => (error as { status?: number }).status === 403);
  assert.equal(f.sockets[0].sent.some((frame) => frame.type === "unsubscribe" && frame.id === deniedId), true,
    "terminal subscription errors release the server-side active slot");
  t.mock.timers.tick(5_000);
  assert.equal(f.sockets[0].sent.filter((frame) => frame.type === "subscribe" && frame.id === deniedId).length, 1);
  assert.deepEqual(errors, [403]);
  transient.unsubscribe();
});

test("promotion resolves a subscription first established on the candidate", async (t) => {
  const f = setup(t);
  const existing = f.gateway.subscribe({ kind: "chat", channelId: "existing", after: "0" }, { event: () => undefined, cursor: () => "0" });
  f.hello(0);
  const existingId = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id: existingId, event: { type: "ready", cursor: "0" } });
  f.sockets[0].frame({ type: "subscribed", id: existingId });
  await existing.ready;
  t.after(() => existing.unsubscribe());

  f.sockets[0].frame({ type: "migrating" });
  const candidateOnly = f.gateway.subscribe({ kind: "chat", channelId: "new", after: "0" }, { event: () => undefined, cursor: () => "0" });
  t.after(() => candidateOnly.unsubscribe());
  f.hello(1);
  for (const frame of f.sockets[1].sent.filter((value) => value.type === "subscribe")) {
    f.sockets[1].frame({ type: "event", id: frame.id, event: { type: "ready", cursor: "0" } });
    f.sockets[1].frame({ type: "subscribed", id: frame.id });
  }
  await candidateOnly.ready;
  assert.equal(f.sockets[0].closed, true);
});

test("handoff waits for a media snapshot at least as new as the applied revision", (t) => {
  const f = setup(t);
  const revisions: number[] = [];
  const subscription = f.gateway.subscribe({ kind: "media" }, {
    event: (event) => revisions.push((event as { revision: number }).revision),
  });
  t.after(() => subscription.unsubscribe());
  f.hello(0);
  const id = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id, event: { type: "snapshot", revision: 5, participants: [] } });
  f.sockets[0].frame({ type: "subscribed", id });
  f.sockets[0].frame({ type: "migrating" });
  f.hello(1);
  f.sockets[1].frame({ type: "event", id, event: { type: "snapshot", revision: 4, participants: [] } });
  f.sockets[1].frame({ type: "subscribed", id });
  assert.equal(f.sockets[0].closed, false);

  f.sockets[0].frame({ type: "event", id, event: { type: "snapshot", revision: 6, participants: [] } });
  f.sockets[1].frame({ type: "event", id, event: { type: "snapshot", revision: 5, participants: [] } });
  assert.equal(f.sockets[0].closed, false);
  f.sockets[1].frame({ type: "event", id, event: { type: "snapshot", revision: 6, participants: [] } });
  assert.equal(f.sockets[0].closed, true);
  assert.deepEqual(revisions, [5, 6]);
});

test("candidate presence snapshots remain hidden until promotion and only the latest is applied", (t) => {
  const f = setup(t);
  const states: string[] = [];
  const subscription = f.gateway.subscribe({ kind: "presence", spaceId: "space", userIds: ["one"] }, {
    event: (event) => states.push((event as { members: Array<{ status: string }> }).members[0].status),
  });
  t.after(() => subscription.unsubscribe());
  f.hello(0);
  const id = f.subscribe(0).id;
  const snapshot = (status: string) => ({ type: "snapshot", members: [{ userId: "one", status }] });
  f.sockets[0].frame({ type: "event", id, event: snapshot("online") });
  f.sockets[0].frame({ type: "subscribed", id });
  f.sockets[0].frame({ type: "migrating" });
  f.hello(1);
  f.sockets[1].frame({ type: "event", id, event: snapshot("offline") });
  f.sockets[1].frame({ type: "event", id, event: snapshot("idle") });
  assert.deepEqual(states, ["online"]);
  f.sockets[0].frame({ type: "event", id, event: snapshot("online") });
  assert.deepEqual(states, ["online", "online"]);
  f.sockets[1].frame({ type: "subscribed", id });
  assert.equal(f.sockets[0].closed, true);
  assert.deepEqual(states, ["online", "online", "idle"]);
});

test("legacy spectator presence reports live status across gateway reconnect", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  setAppGatewayForTests(f.gateway);
  t.after(() => setAppGatewayForTests(undefined));
  const live: boolean[] = [];
  const stop = watchMediaPresence(() => undefined, (online) => live.push(online));
  t.after(stop);
  f.hello(0);
  const id = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id, event: { type: "snapshot", revision: 1, participants: [] } });
  f.sockets[0].frame({ type: "subscribed", id });
  f.sockets[0].fail();
  t.mock.timers.tick(188);
  f.hello(1);
  f.sockets[1].frame({ type: "event", id, event: { type: "snapshot", revision: 2, participants: [] } });
  f.sockets[1].frame({ type: "subscribed", id });
  assert.deepEqual(live, [false, true, false, true]);
});

test("presence subscriptions reject more than 100 users", (t) => {
  const f = setup(t);
  assert.throws(() => f.gateway.subscribe({
    kind: "presence", spaceId: "space", userIds: Array.from({ length: 101 }, (_, index) => String(index)),
  }, { event: () => undefined }), /at most 100/);
  assert.equal(f.sockets.length, 0);
});

test("public watchPresence delivers scoped member states", (t) => {
  const f = setup(t);
  setAppGatewayForTests(f.gateway);
  t.after(() => setAppGatewayForTests(undefined));
  const received: unknown[] = [];
  const stop = watchPresence("space", ["one"], (members) => received.push(members));
  t.after(stop);
  f.hello(0);
  const id = f.subscribe(0).id;
  f.sockets[0].frame({ type: "event", id, event: { type: "snapshot", members: [{ userId: "one", status: "idle" }] } });
  f.sockets[0].frame({ type: "subscribed", id });
  assert.deepEqual(received, [[{ userId: "one", status: "idle" }]]);
});
