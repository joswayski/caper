import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { EventConnection } from "../media/event-connection.ts";
import type { CallSnapshot } from "../media/types.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const frame = (name: string, value: unknown = {}) => new TextEncoder().encode(`event: ${name}\ndata: ${JSON.stringify(value)}\n\n`);

function setup(t: TestContext, presence = false) {
  const streams: Array<{ controller: ReadableStreamDefaultController<Uint8Array>; signal: AbortSignal }> = [];
  const values: Array<CallSnapshot & { revision?: number }> = [];
  const errors: Error[] = [];
  let drains = 0;
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, presence ? "/api/media/presence/events?handoff=1" : "/api/media/events?snapshots=1&handoff=1");
    assert.equal(new Headers(init.headers).get("x-caper-media-token"), presence ? null : "test-token");
    const signal = init.signal!;
    let controller!: ReadableStreamDefaultController<Uint8Array>;
    const abort = () => controller.error(signal.reason);
    return new Response(new ReadableStream<Uint8Array>({
      start(c) { controller = c; streams.push({ controller, signal }); signal.addEventListener("abort", abort, { once: true }); },
      cancel() { signal.removeEventListener("abort", abort); },
    }), { headers: { "content-type": "text/event-stream" } });
  });
  const owner = new AbortController();
  const connection = new EventConnection(() => assert.fail("expected snapshots"), (error) => errors.push(error), (value) => values.push(value), () => drains++);
  t.after(() => { owner.abort(); connection.stop(); });
  const send = (index: number, name: string, data?: unknown) => streams[index].controller.enqueue(frame(name, data));
  const snapshot = (index: number, revision: number, muted: boolean, deafened: boolean) => send(index, "snapshot", {
    revision, participants: [{ id: "phone", name: "Phone", muted, deafened, ...(presence ? {} : { tracks: [] }) }],
  });
  const start = async () => {
    const pending = presence ? connection.openPresence(owner.signal) : connection.open("test-token", owner.signal);
    await tick();
    send(0, "ready"); snapshot(0, 1, false, false);
    await pending; await tick();
  };
  return { connection, owner, streams, values, errors, send, snapshot, start, drains: () => drains };
}

for (const presence of [false, true]) {
  test(`${presence ? "spectator" : "authenticated"} handoff keeps every pushed state flowing until a current replacement snapshot`, async (t) => {
    const f = setup(t, presence);
    await f.start();
    let revision = 1;
    for (let cycle = 0; cycle < 3; cycle++) {
      f.send(cycle, "migrating");
      f.send(cycle, "migrating"); // Duplicate notice must not open a third stream.
      await tick();
      assert.equal(f.streams.length, cycle + 2);
      f.send(cycle + 1, "ready");
      await tick();
      assert.equal(f.streams[cycle].signal.aborted, false, "ready alone cannot retire the old stream");
      for (const [muted, deafened] of [[true, false], [true, true], [false, false], [false, true]]) {
        f.snapshot(cycle, ++revision, muted, deafened);
        await tick();
        assert.equal(f.values.at(-1)?.revision, revision, "updates must arrive during the pending handoff");
        assert.equal(f.values.at(-1)?.participants[0].muted, muted);
        assert.equal(f.values.at(-1)?.participants[0].deafened, deafened);
        assert.equal(f.connection.connected, true);
      }
      f.snapshot(cycle + 1, revision - 1, true, false);
      await tick();
      assert.equal(f.values.at(-1)?.revision, revision, "a stale candidate must not regress state");
      assert.equal(f.streams[cycle].signal.aborted, false);
      f.snapshot(cycle + 1, revision, false, true);
      await tick();
      assert.equal(f.streams[cycle].signal.aborted, true, "current snapshot completes handoff");
      assert.equal(f.streams[cycle + 1].signal.aborted, false);
      f.snapshot(cycle + 1, ++revision, true, false);
      await tick();
      assert.equal(f.values.at(-1)?.revision, revision);
    }
    assert.equal(f.drains(), 0);
    assert.deepEqual(f.errors, []);
  });
}

test("a rejected or snapshot-stalled replacement never closes a healthy old stream", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  await f.start();
  const original = fetch;
  let rejected = false;
  t.mock.method(globalThis, "fetch", async (...args: Parameters<typeof fetch>) => {
    if (!rejected) { rejected = true; return Response.json({ code: "api_draining" }, { status: 503 }); }
    return original(...args);
  });
  f.send(0, "migrating"); await tick();
  f.snapshot(0, 2, true, false); await tick();
  assert.equal(f.values.at(-1)?.revision, 2);
  assert.equal(f.streams[0].signal.aborted, false);
  t.mock.timers.tick(50); await tick();
  f.send(1, "ready"); await tick();
  t.mock.timers.tick(4_999); await tick();
  assert.equal(f.streams[1].signal.aborted, false);
  t.mock.timers.tick(1); await tick();
  assert.equal(f.streams[1].signal.aborted, true, "ready without snapshot cannot hang forever");
  f.snapshot(0, 3, false, true); await tick();
  assert.equal(f.values.at(-1)?.revision, 3);
  assert.equal(f.connection.connected, true);
  t.mock.timers.tick(500); await tick();
  f.send(2, "ready"); f.snapshot(2, 3, false, true); await tick();
  assert.equal(f.streams[0].signal.aborted, true);
  assert.equal(f.connection.connected, true);
  assert.deepEqual(f.errors, []);
});

for (const notice of ["migrating", "draining"]) {
  test(`a replacement receiving ${notice} preserves the old stream and cancels its pending retry on leave`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const f = setup(t);
    await f.start();
    f.send(0, "migrating"); await tick();
    f.send(1, "ready"); f.send(1, notice); await tick();
    assert.equal(f.streams[1].signal.aborted, true);
    assert.equal(f.streams[0].signal.aborted, false);
    f.snapshot(0, 2, false, true); await tick();
    assert.equal(f.values.at(-1)?.revision, 2);
    assert.equal(f.drains(), 0, "candidate shutdown is not a logical subscription outage");
    assert.deepEqual(f.errors, []);
    t.mock.timers.tick(49); await tick();
    assert.equal(f.streams.length, 2);
    f.owner.abort();
    t.mock.timers.tick(30_000); await tick();
    assert.equal(f.streams.length, 2, "leave must cancel the scheduled candidate retry");
    assert.equal(f.streams.every((stream) => stream.signal.aborted), true);
  });
}

test("old EOF does not cancel a pending replacement; failure of both returns to outage recovery", async (t) => {
  const f = setup(t);
  await f.start();
  f.send(0, "migrating"); await tick();
  f.streams[0].controller.close(); await tick();
  assert.deepEqual(f.errors, []);
  f.send(1, "ready"); f.snapshot(1, 2, true, false); await tick();
  assert.equal(f.connection.connected, true);
  f.send(1, "migrating"); await tick();
  f.streams[1].controller.close(); await tick();
  f.streams[2].controller.error(new Error("replacement failed")); await tick();
  assert.equal(f.errors.length, 1);
  assert.equal(f.connection.connected, false);
});

test("cancellation closes both overlapping streams and suppresses buffered snapshots and retries", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup(t);
  await f.start();
  f.send(0, "migrating"); await tick();
  const count = f.values.length;
  f.send(1, "ready"); f.snapshot(1, 2, true, false);
  f.owner.abort();
  await tick();
  t.mock.timers.tick(30_000); await tick();
  assert.equal(f.values.length, count);
  assert.equal(f.streams.length, 2);
  assert.equal(f.streams.every((stream) => stream.signal.aborted), true);
  assert.deepEqual(f.errors, []);
});
