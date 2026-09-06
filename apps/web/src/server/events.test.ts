import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { CallEvents } from "../media/events.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function setup(t: TestContext) {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let signal!: AbortSignal;
  let changes = 0;
  const errors: Error[] = [];
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "/api/media/events", "capability must never enter a URL");
    assert.equal(new Headers(init.headers).get("authorization"), "Bearer test-capability");
    signal = init.signal!;
    const abort = () => controller.error(signal.reason);
    return new Response(new ReadableStream<Uint8Array>({
      start(value) { controller = value; signal.addEventListener("abort", abort, { once: true }); },
      cancel() { signal.removeEventListener("abort", abort); },
    }), { headers: { "content-type": "text/event-stream" } });
  });
  const events = new CallEvents(() => changes++, (error) => errors.push(error));
  const owner = new AbortController();
  t.after(() => { owner.abort(); events.stop(); });
  return {
    events, owner, errors,
    send: (value: string) => controller.enqueue(new TextEncoder().encode(value)),
    changes: () => changes,
    aborted: () => signal.aborted,
  };
}

test("SSE waits for a complete ready event, supports chunk boundaries/CRLF, and ignores unknown events", async (t) => {
  const { events, owner, send, changes } = setup(t);
  let ready = false;
  const opening = events.open("test-capability", owner.signal).then(() => { ready = true; });
  await tick();
  assert.equal(ready, false);
  send(": comment\n\nevent: future\ndata: {}\n\nevent: rea");
  await tick();
  assert.equal(ready, false);
  send("dy\r\ndata: {}\r\n\r");
  await tick();
  assert.equal(ready, false);
  send("\nevent: changed\ndata: {}\n\nevent: heartbeat\ndata: {}\n\n");
  await opening;
  await tick();
  assert.equal(events.connected, true);
  assert.equal(changes(), 1);
});

test("SSE startup timeout and silent stream timeout fail readiness", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { events, owner, errors, aborted } = setup(t);
  const opening = events.open("test-capability", owner.signal);
  const failed = assert.rejects(opening, /timed out/);
  await tick();
  t.mock.timers.tick(10_000);
  await failed;
  assert.equal(events.connected, false);
  assert.equal(aborted(), true);
  assert.equal(errors.length, 1);
});

test("heartbeat refreshes the watchdog; a missing heartbeat closes a ready stream", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { events, owner, send, errors } = setup(t);
  const opening = events.open("test-capability", owner.signal);
  await tick();
  send("event: ready\ndata: {}\n\n");
  await opening;
  t.mock.timers.tick(20_000);
  send("event: heartbeat\ndata: {}\n\n");
  await tick();
  t.mock.timers.tick(20_000);
  assert.equal(events.connected, true);
  t.mock.timers.tick(5_000);
  await tick();
  assert.equal(events.connected, false);
  assert.equal(errors.length, 1);
});

test("owner cancellation rejects pending readiness without reporting a connection failure", async (t) => {
  const { events, owner, errors } = setup(t);
  const opening = events.open("test-capability", owner.signal);
  const failed = assert.rejects(opening, { name: "AbortError" });
  await tick();
  owner.abort();
  await failed;
  assert.equal(errors.length, 0);
  assert.equal(events.connected, false);
});

test("an already cancelled join does not open an event stream", async (t) => {
  const { events, owner, errors } = setup(t);
  const fetchSpy = t.mock.method(globalThis, "fetch");
  owner.abort();
  await assert.rejects(events.open("test-capability", owner.signal), { name: "AbortError" });
  assert.equal(fetchSpy.mock.callCount(), 0);
  assert.equal(events.connected, false);
  assert.equal(errors.length, 0);
});

test("changed before ready and oversized partial frames are rejected", async (t) => {
  for (const input of ["event: changed\ndata: {}\n\n", "x".repeat(65_537)]) {
    await t.test(input.length > 100 ? "oversized" : "missing handshake", async (t) => {
      const { events, owner, send } = setup(t);
      const failed = assert.rejects(events.open("test-capability", owner.signal), /Invalid live update/);
      await tick();
      send(input);
      await failed;
      assert.equal(events.connected, false);
    });
  }
});
