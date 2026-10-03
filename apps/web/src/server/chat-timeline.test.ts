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

test("sorted snapshots are reused until visible messages change and never mutate previous snapshots", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("12"), message("10")], "12");
  const initial = timeline.messages;
  assert.deepEqual(initial.map((value) => value.seq), ["10", "12"]);
  assert.equal(timeline.messages, initial, "reading must not repeatedly allocate and sort history");
  timeline.applyEvent(message("12"));
  timeline.applyEvent(message("14"));
  assert.equal(timeline.messages, initial, "duplicates and buffered events leave the visible snapshot intact");
  timeline.prepend([message("9")]);
  const paginated = timeline.messages;
  assert.deepEqual(paginated.map((value) => value.seq), ["9", "10", "12"]);
  timeline.mergeSent(message("15"));
  assert.deepEqual(timeline.messages.map((value) => value.seq), ["9", "10", "12", "15"]);
  timeline.applyEvent(message("13"));
  assert.deepEqual(timeline.messages.map((value) => value.seq), ["9", "10", "12", "13", "14", "15"]);
  assert.deepEqual(initial.map((value) => value.seq), ["10", "12"]);
  assert.deepEqual(paginated.map((value) => value.seq), ["9", "10", "12"]);
  timeline.reset([], "0");
  assert.deepEqual(timeline.messages, [], "reset must invalidate even when no messages are merged");
});
