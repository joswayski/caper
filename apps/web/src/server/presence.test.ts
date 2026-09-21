import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { CallEvents } from "../media/events.ts";
import { watchPresence } from "../media/presence.ts";
import type { CallSnapshot } from "../media/types.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const encoder = new TextEncoder();
const participant = { id: "self", name: "Caper", muted: false, deafened: false };
const event = (name: string, data: unknown = {}) => encoder.encode(
  `event: ${name}\ndata: ${JSON.stringify(data)}\n\n`,
);

interface StreamHandle {
  controller: ReadableStreamDefaultController<Uint8Array>;
  signal: AbortSignal;
}

function streams(t: TestContext, failures: number[] = []) {
  const opened: StreamHandle[] = [];
  const requests: Array<{ url: string; init: RequestInit }> = [];
  let call = 0;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = String(input);
    requests.push({ url, init });
    assert.equal(url, "/api/media/presence/events");
    const headers = new Headers(init.headers);
    assert.equal(headers.get("authorization"), null, "public presence must not send authentication");
    assert.equal(headers.get("x-caper-media-token"), null, "public presence must not send the media capability");
    assert.equal(headers.get("accept"), "text/event-stream");
    assert.equal(init.cache, "no-store");
    if (failures.includes(call++)) throw new Error("startup failed");
    const signal = init.signal!;
    let handle!: StreamHandle;
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        handle = { controller, signal };
        opened.push(handle);
      },
    });
    return new Response(body, { headers: { "content-type": "text/event-stream" } });
  });
  return { opened, requests };
}

test("presence uses only the unauthenticated stream and a pushed empty snapshot clears stale self immediately", async (t) => {
  const mock = streams(t);
  const snapshots: CallSnapshot[] = [];
  const live: boolean[] = [];
  const stop = watchPresence((value) => snapshots.push(value), (value) => live.push(value));
  t.after(stop);
  await tick();

  mock.opened[0].controller.enqueue(event("ready"));
  mock.opened[0].controller.enqueue(event("snapshot", { participants: [participant], revision: 1 }));
  await tick();
  assert.deepEqual(snapshots, [{ participants: [{ ...participant, tracks: [] }], revision: 1 }]);
  assert.deepEqual(live, [false, true]);

  mock.opened[0].controller.enqueue(event("snapshot", { participants: [], revision: 2 }));
  await tick();
  assert.deepEqual(snapshots.at(-1), { participants: [], revision: 2 }, "leave is applied from the push without a polling interval");
  assert.equal(mock.requests.length, 1, "presence must not issue JSON snapshot polling requests");
});

test("startup failure and EOF retry, while a successful snapshot resets liveness and retry backoff", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const mock = streams(t, [0]);
  const snapshots: CallSnapshot[] = [];
  const live: boolean[] = [];
  const stop = watchPresence((value) => snapshots.push(value), (value) => live.push(value));
  t.after(stop);
  await tick();
  assert.equal(mock.requests.length, 1);
  assert.equal(live.at(-1), false);

  t.mock.timers.tick(249);
  await tick();
  assert.equal(mock.requests.length, 1);
  t.mock.timers.tick(1);
  await tick();
  assert.equal(mock.requests.length, 2, "startup failures reconnect");
  mock.opened[0].controller.enqueue(event("ready"));
  mock.opened[0].controller.enqueue(event("snapshot", { participants: [participant] }));
  await tick();
  assert.equal(live.at(-1), true);

  mock.opened[0].controller.close();
  await tick();
  assert.equal(live.at(-1), false, "EOF marks a previously live stream unavailable");
  t.mock.timers.tick(249);
  await tick();
  assert.equal(mock.requests.length, 2);
  t.mock.timers.tick(1);
  await tick();
  assert.equal(mock.requests.length, 3, "a delivered snapshot resets failure backoff to 250ms");
  mock.opened[1].controller.enqueue(event("ready"));
  mock.opened[1].controller.enqueue(event("snapshot", { participants: [], revision: 3 }));
  await tick();
  assert.deepEqual(snapshots.at(-1), { participants: [], revision: 3 });
  assert.equal(live.at(-1), true);
});

test("draining reopens promptly, requires a fresh snapshot, and cancellation suppresses late work", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const mock = streams(t);
  const snapshots: CallSnapshot[] = [];
  const live: boolean[] = [];
  const stop = watchPresence((value) => snapshots.push(value), (value) => live.push(value));
  await tick();
  mock.opened[0].controller.enqueue(event("ready"));
  mock.opened[0].controller.enqueue(event("snapshot", { participants: [participant], revision: 8 }));
  await tick();

  mock.opened[0].controller.enqueue(event("draining"));
  await tick();
  assert.equal(live.at(-1), false);
  assert.equal(mock.requests.length, 1);
  t.mock.timers.tick(50);
  await tick();
  assert.equal(mock.requests.length, 2, "planned draining bypasses failure backoff");
  assert.equal(live.at(-1), false, "reopening alone does not restore live status");
  mock.opened[1].controller.enqueue(event("ready"));
  await tick();
  assert.equal(live.at(-1), false, "ready is not a fresh roster");
  mock.opened[1].controller.enqueue(event("snapshot", { participants: [], revision: 9 }));
  await tick();
  assert.deepEqual(snapshots.at(-1), { participants: [], revision: 9 });
  assert.equal(live.at(-1), true);

  const snapshotCount = snapshots.length;
  mock.opened[1].controller.enqueue(event("snapshot", { participants: [participant], revision: 10 }));
  stop();
  assert.equal(mock.opened[1].signal.aborted, true);
  t.mock.timers.tick(30_000);
  await tick();
  assert.equal(mock.requests.length, 2, "cancelled watchers never reopen");
  assert.equal(snapshots.length, snapshotCount, "cancelled watchers cannot publish late updates");
});

test("spectator draining rejections use a bounded fast retry budget", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const mock = streams(t);
  const original = fetch;
  let attempts = 0;
  let rejecting = true;
  t.mock.method(globalThis, "fetch", async (...args: Parameters<typeof fetch>) => {
    if (rejecting) {
      attempts++;
      return Response.json({ code: "api_draining" }, { status: 503 });
    }
    return original(...args);
  });
  const snapshots: CallSnapshot[] = [];
  const stop = watchPresence((value) => snapshots.push(value), () => undefined);
  t.after(stop);
  await tick();
  for (let i = 0; i < 10; i++) {
    t.mock.timers.tick(50);
    await tick();
  }
  assert.equal(attempts, 11);
  t.mock.timers.tick(249);
  await tick();
  assert.equal(attempts, 11, "fast retries must be bounded during an outage");
  rejecting = false;
  t.mock.timers.tick(1);
  await tick();
  assert.equal(mock.opened.length, 1);
  mock.opened[0].controller.enqueue(event("ready"));
  mock.opened[0].controller.enqueue(event("snapshot", { participants: [participant], revision: 4 }));
  await tick();
  assert.equal(snapshots[0].participants[0].id, "self");
  rejecting = true;
  mock.opened[0].controller.enqueue(event("draining"));
  await tick();
  t.mock.timers.tick(50);
  await tick();
  assert.equal(attempts, 12, "a fresh roster resets the drain retry budget");
  stop();
  t.mock.timers.tick(10_000);
  await tick();
  assert.equal(attempts, 12, "cancellation must stop scheduled retries");
});

test("public presence rejects snapshots containing private authenticated track data", async (t) => {
  const mock = streams(t);
  const errors: Error[] = [];
  const snapshots: CallSnapshot[] = [];
  const events = new CallEvents(() => assert.fail("public presence cannot invalidate"), (error) => errors.push(error), (value) => snapshots.push(value));
  const owner = new AbortController();
  t.after(() => { owner.abort(); events.stop(); });
  const opening = events.openPresence(owner.signal);
  await tick();
  mock.opened[0].controller.enqueue(event("ready"));
  await opening;
  mock.opened[0].controller.enqueue(event("snapshot", {
    participants: [{ ...participant, tracks: [{ id: "private-track-id", kind: "microphone" }] }],
  }));
  await tick();
  assert.equal(events.connected, false);
  assert.equal(snapshots.length, 0);
  assert.match(errors[0]?.message ?? "", /Invalid live update snapshot/);
});
