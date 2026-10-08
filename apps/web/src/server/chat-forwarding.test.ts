import assert from "node:assert/strict";
import { test } from "vitest";
import { ChatTimeline } from "../chat/timeline.ts";
import { isChatForwardEvent, isChatMessage, type ChatForwardEvent, type ChatMessage } from "../chat/types.ts";

function forward(sourceSeq = "9007199254740993", destinationSeq = "2", id = "wrapper"): ChatMessage {
  const original: ChatMessage = { id: "original", channelId: "private-source", seq: "89", author: { id: "alice", name: "Alice", isGuest: false },
    content: { version: 1, type: "text", text: `source ${sourceSeq}` }, createdAt: "2026-10-06T10:00:00Z", clientMessageId: "source-key",
    thread: { replyCount: 7, participants: [], seq: sourceSeq }, revision: 3, editedAt: "2026-10-06T11:00:00Z", editSeq: "91" };
  return { id, channelId: "destination", seq: "1", author: { id: "bob", name: "Bob", isGuest: false },
    content: { version: 1, type: "text", text: "my note" }, createdAt: "2026-10-06T11:01:00Z", clientMessageId: `key-${id}`,
    forward: { message: original, seq: sourceSeq }, forwardSeq: destinationSeq };
}
const event = (sourceSeq: string, destinationSeq: string, id = "wrapper"): ChatForwardEvent => ({
  type: "message.forward", schemaVersion: 1, channelId: "destination", seq: destinationSeq, message: forward(sourceSeq, destinationSeq, id),
});

test("forward updates fill destination gaps without using the source cursor or moving the wrapper", () => {
  const timeline = new ChatTimeline();
  timeline.reset([forward("9007199254740992", "1")], "1");
  assert.equal(timeline.applyEvent(event("9007199254740994", "3")), "buffered");
  assert.equal(timeline.cursor, "1");
  assert.equal(timeline.applyEvent(event("9007199254740993", "2")), "applied");
  assert.equal(timeline.cursor, "3");
  assert.equal(timeline.messages.length, 1);
  assert.equal(timeline.messages[0].seq, "1");
  assert.equal(timeline.messages[0].content.text, "my note");
  assert.equal(timeline.messages[0].forward?.message?.content.text, "source 9007199254740994");
});

test("fresher hydrated sources survive later destination events, stale sends and pinned history", () => {
  const timeline = new ChatTimeline();
  const fresh = forward("9007199254740995", "1");
  timeline.reset([fresh], "1");
  timeline.applyEvent(event("9007199254740993", "2"));
  timeline.mergeSent(forward("9007199254740992", "1"));
  assert.equal(timeline.messages[0].forward?.seq, "9007199254740995");
  assert.equal(timeline.messages[0].forwardSeq, "2");
  assert.equal(timeline.cursor, "2");
  timeline.reset([forward("9007199254740993", "2")], "2", [{ ...fresh, pinSeq: "1", pin: { author: fresh.author, createdAt: fresh.createdAt } }]);
  assert.equal(timeline.messages[0].forward?.seq, "9007199254740995");
  assert.equal(timeline.pinnedMessages[0].forward?.seq, "9007199254740995");
  timeline.applyEvent(event("9007199254740996", "3"));
  assert.equal(timeline.pinnedMessages[0].forward?.seq, "9007199254740996");
});

test("updates to unloaded wrappers remain scoped overlays until pagination loads the message", () => {
  const timeline = new ChatTimeline();
  timeline.reset([], "10");
  timeline.applyEvent(event("9007199254740994", "11", "old"));
  assert.equal(timeline.messages.length, 0);
  timeline.prepend([forward("9007199254740993", "1", "old")]);
  assert.equal(timeline.messages[0].forward?.seq, "9007199254740994");
  const removed = { ...forward("0", "12", "old"), forward: { message: null, seq: "0" } };
  timeline.applyEvent({ ...event("0", "12", "old"), message: removed });
  timeline.mergeSent(forward("9007199254740993", "1", "old"));
  assert.equal(timeline.messages[0].forward?.message, null, "stale acknowledgements cannot resurrect an unavailable original");
  timeline.reset([], "0");
  timeline.prepend([forward("9007199254740992", "1", "old")]);
  assert.notEqual(timeline.messages[0].forward?.message, null, "access reset discards old scoped overlays");
});

test("forward wire validation rejects cross-channel and revision mismatches, malformed originals and chains", () => {
  const valid = event("9007199254740993", "2");
  assert.ok(isChatForwardEvent(valid));
  assert.ok(isChatMessage({ ...valid.message, forward: { seq: "0", message: null } }));
  assert.equal(isChatForwardEvent({ ...valid, channelId: "source" }), false);
  assert.equal(isChatForwardEvent({ ...valid, seq: "3" }), false);
  assert.equal(isChatForwardEvent({ ...valid, schemaVersion: 2 }), false);
  assert.equal(isChatMessage({ ...valid.message, forward: { seq: "02", message: null } }), false);
  assert.equal(isChatMessage({ ...valid.message, forward: { seq: "2", message: { content: { text: "untrusted" } } } }), false);
  assert.equal(isChatMessage({ ...valid.message, forward: { seq: "2", message: forward() } }), false);
});

test("projected source edits and destination reply edits preserve independent thread, reaction and pin state", () => {
  const wrapper = { ...forward("99", "1"), reactionSeq: "5", reactions: [{ emoji: "👀", authorIds: ["peer"] }],
    pinSeq: "4", pin: { author: forward().author, createdAt: "2026-10-06T12:00:00Z" },
    thread: { replyCount: 1, participants: [forward().author], seq: "3" } };
  const reply: ChatMessage = { ...wrapper, id: "local-reply", seq: "2", clientMessageId: "reply-key", forward: undefined,
    forwardSeq: undefined, pin: undefined, pinSeq: undefined, threadRootId: wrapper.id, content: { version: 1, type: "text", text: "My local reply" } };
  const timeline = new ChatTimeline();
  timeline.reset([wrapper, reply], "5", [wrapper]);
  const projected = event("101", "6");
  projected.message.forward!.message = { ...projected.message.forward!.message!, revision: 4, editSeq: "101",
    content: { version: 1, type: "text", text: "Edited original" } };
  timeline.applyEvent(projected);
  timeline.applyEvent({ type: "message.edited", schemaVersion: 1, channelId: "destination", seq: "7",
    message: { ...reply, revision: 2, editedAt: "2026-10-06T12:30:00Z", editSeq: "7", content: { version: 1, type: "text", text: "Edited local reply" } } });
  timeline.mergeSent(wrapper);
  assert.equal(timeline.cursor, "7");
  const current = timeline.messages.find((message) => message.id === wrapper.id)!;
  assert.equal(current.content.text, "my note");
  assert.equal(current.forward?.message?.content.text, "Edited original");
  assert.equal(current.forward?.message?.revision, 4);
  assert.deepEqual(current.thread, wrapper.thread);
  assert.deepEqual(current.reactions, wrapper.reactions);
  assert.deepEqual(current.pin, wrapper.pin);
  assert.equal(timeline.pinnedMessages[0].forward?.message?.content.text, "Edited original");
  assert.equal(timeline.messages.find((message) => message.id === reply.id)!.content.text, "Edited local reply");
});
