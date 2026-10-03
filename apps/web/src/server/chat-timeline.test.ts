import assert from "node:assert/strict";
import test from "node:test";
import { ChatTimeline } from "../chat/timeline.ts";
import type { ChatMessage } from "../chat/types.ts";

function message(seq: string, id = `message-${seq}`): ChatMessage {
  return {
    id, channelId: "general", seq, author: { id: "guest", name: "Guest", isGuest: true },
    content: { version: 1, type: "text", text: `<b>safe ${seq}</b>` }, createdAt: "2026-09-21T12:00:00Z", clientMessageId: `client-${seq}`,
  };
}

test("overlap is deduplicated and gaps do not advance the durable BigInt cursor", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("9007199254740992")], "9007199254740992");

  assert.equal(timeline.applyEvent(message("9007199254740994")), "buffered");
  assert.equal(timeline.cursor, "9007199254740992", "a highest-seen event must not skip a gap");
  assert.equal(timeline.applyEvent(message("9007199254740993")), "applied");
  assert.equal(timeline.cursor, "9007199254740994");
  assert.equal(timeline.applyEvent(message("9007199254740993")), "duplicate");
  assert.deepEqual(timeline.messages.map((value) => value.seq), ["9007199254740992", "9007199254740993", "9007199254740994"]);
});

test("an HTTP send merges immediately but only its matching event advances replay", () => {
  const timeline = new ChatTimeline();
  timeline.reset([], "5");
  const sent = message("6", "durable-send");

  timeline.mergeSent(sent);
  assert.equal(timeline.cursor, "5");
  assert.deepEqual(timeline.messages.map((value) => value.id), ["durable-send"]);

  assert.equal(timeline.applyEvent(sent), "applied");
  assert.equal(timeline.cursor, "6");
  assert.equal(timeline.messages.length, 1, "the event deduplicates the HTTP response by message id");
});

test("message snapshots retain identity until visible contents change", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1");
  const snapshot = timeline.messages;

  assert.strictEqual(timeline.messages, snapshot);
  assert.equal(timeline.applyEvent(message("1")), "duplicate");
  assert.strictEqual(timeline.messages, snapshot, "duplicate events do not invalidate the snapshot");
  assert.equal(timeline.applyEvent(message("3")), "buffered");
  assert.strictEqual(timeline.messages, snapshot, "buffered events are not visible yet");
});

test("prepend, send, gap draining, and reset invalidate message snapshots", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("2")], "2");

  const initial = timeline.messages;
  timeline.prepend([message("1")]);
  const prepended = timeline.messages;
  assert.notStrictEqual(prepended, initial);

  timeline.mergeSent(message("5"));
  const sent = timeline.messages;
  assert.notStrictEqual(sent, prepended);

  assert.equal(timeline.applyEvent(message("4")), "buffered");
  assert.strictEqual(timeline.messages, sent);
  assert.equal(timeline.applyEvent(message("3")), "applied");
  const drained = timeline.messages;
  assert.notStrictEqual(drained, sent);
  assert.deepEqual(drained.map(({ seq }) => seq), ["1", "2", "3", "4", "5"]);

  timeline.reset([], "9");
  assert.notStrictEqual(timeline.messages, drained);
  assert.deepEqual(timeline.messages, []);
});

test("cached snapshots preserve BigInt ordering, id tie breaks, and immutability", () => {
  const timeline = new ChatTimeline();
  timeline.reset([
    message("9007199254740993", "z"),
    message("9007199254740992", "middle"),
    message("9007199254740993", "a"),
  ], "9007199254740993");
  const snapshot = timeline.messages;

  assert.deepEqual(snapshot.map(({ id }) => id), ["middle", "a", "z"]);
  timeline.mergeSent(message("9007199254740994", "later"));
  assert.deepEqual(snapshot.map(({ id }) => id), ["middle", "a", "z"], "previous snapshots must not mutate");
  assert.deepEqual(timeline.messages.map(({ id }) => id), ["middle", "a", "z", "later"]);
});
