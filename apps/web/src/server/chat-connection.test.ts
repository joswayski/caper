import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { ChatConnection } from "../chat/connection.ts";
import { ChatTimeline } from "../chat/timeline.ts";
import type { ChatMessage } from "../chat/types.ts";

class FakeSocket {
  closed = false;
  readonly url: string;
  private readonly listeners = new Map<string, Array<(event: Event | MessageEvent) => void>>();
  constructor(url: string) { this.url = url; }
  addEventListener(type: string, listener: (event: Event | MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() { this.closed = true; }
  frame(value: unknown) { this.emit("message", { data: JSON.stringify(value) } as MessageEvent); }
  fail() { this.emit("close", new Event("close")); }
  private emit(type: string, event: Event | MessageEvent) { for (const listener of this.listeners.get(type) ?? []) listener(event); }
}

function message(seq: string): ChatMessage {
  return {
    id: `message-${seq}`, channelId: "general", seq, author: { id: "guest", name: "Guest", isGuest: true },
    content: { version: 1, type: "text", text: seq }, createdAt: "2026-09-21T12:00:00Z", clientMessageId: `client-${seq}`,
  };
}

function setup(t: TestContext) {
  const originalWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", { configurable: true, value: { location: { protocol: "https:", host: "caper.test" } } });
  t.after(() => Object.defineProperty(globalThis, "window", { configurable: true, value: originalWindow }));
  const timeline = new ChatTimeline();
  timeline.reset([], "5");
  const sockets: FakeSocket[] = [];
  const statuses: boolean[] = [];
  let resyncs = 0;
  const connection = new ChatConnection("general", {
    cursor: () => timeline.cursor,
    message: (value) => timeline.applyEvent(value),
    status: (online) => statuses.push(online),
    resync: () => resyncs++,
  }, (url) => { const socket = new FakeSocket(url); sockets.push(socket); return socket; }, () => 0);
  t.after(() => connection.stop());
  connection.start();
  return { connection, timeline, sockets, statuses, resyncs: () => resyncs };
}

test("a stale ready during migration retains the old socket until the candidate catches up", (t) => {
  const value = setup(t);
  value.sockets[0].frame({ type: "ready", cursor: "5" });
  value.sockets[0].frame({ type: "migrating" });
  assert.equal(value.sockets.length, 2);
  assert.match(value.sockets[1].url, /after=5/);

  value.sockets[0].frame({ type: "message.created", channelId: "general", seq: "6", message: message("6") });
  value.sockets[1].frame({ type: "ready", cursor: "5" });
  assert.equal(value.sockets[0].closed, false, "stale ready must not drop the still-current old stream");

  value.sockets[1].frame({ type: "message.created", channelId: "general", seq: "6", message: message("6") });
  assert.equal(value.sockets[0].closed, true, "candidate is promotable after receiving the overlap through the current cursor");
  assert.equal(value.sockets[1].closed, false);
  assert.equal(value.timeline.cursor, "6");
  assert.equal(value.resyncs(), 0);
});

test("candidate failure leaves a healthy old stream delivering events", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const value = setup(t);
  value.sockets[0].frame({ type: "ready", cursor: "5" });
  value.sockets[0].frame({ type: "migrating" });
  value.sockets[1].fail();

  assert.equal(value.sockets[0].closed, false);
  value.sockets[0].frame({ type: "message.created", channelId: "general", seq: "6", message: message("6") });
  assert.equal(value.timeline.cursor, "6");
  assert.deepEqual(value.statuses, [true], "candidate failure is not an offline transition while old is healthy");
  assert.equal(value.resyncs(), 0);
});

test("a per-socket sequence gap requests bounded initial resynchronization", (t) => {
  const value = setup(t);
  value.sockets[0].frame({ type: "ready", cursor: "5" });
  value.sockets[0].frame({ type: "message.created", channelId: "general", seq: "7", message: message("7") });
  assert.equal(value.timeline.cursor, "5");
  assert.equal(value.resyncs(), 1);
});
