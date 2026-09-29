import assert from "node:assert/strict";
import test from "node:test";
import { ChatTimeline } from "../chat/timeline.ts";
import { isChatReactionEvent, type ChatMessage, type ChatReactionEvent } from "../chat/types.ts";
import { emojiAsset, emojiCode } from "../chat/emoji.ts";

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

function reaction(seq: string, authorIds = ["other"], messageId = "message-1"): ChatReactionEvent {
  return { type: "message.reactions", schemaVersion: 1, channelId: "general", messageId, seq,
    reactions: authorIds.length ? [{ emoji: "👍🏽", authorIds }] : [] };
}

test("reactions fill sequence gaps without becoming messages or accepting stale acknowledgements", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1");
  assert.equal(timeline.applyEvent(message("3")), "buffered");
  timeline.mergeReactions(reaction("4", ["other", "guest"]));
  assert.equal(timeline.cursor, "1", "HTTP reactions cannot skip missing events");
  assert.equal(timeline.applyEvent(reaction("2")), "applied");
  assert.equal(timeline.cursor, "3");
  assert.deepEqual(timeline.messages.map((m) => m.seq), ["1", "3"]);
  assert.deepEqual(timeline.messages[0].reactions, reaction("4", ["other", "guest"]).reactions);
  timeline.applyEvent(reaction("4", ["other", "guest"]));
  timeline.applyEvent(reaction("5", []));
  timeline.mergeReactions(reaction("4", ["other", "guest"]));
  timeline.prepend([{ ...message("1"), reactions: reaction("2").reactions, reactionSeq: "2" }]);
  assert.deepEqual(timeline.messages[0].reactions, [], "late HTTP and paginated snapshots cannot resurrect a removed reaction");
  assert.equal(timeline.cursor, "5");
});

test("reactions to unloaded history survive a stale in-flight page without manufacturing messages", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("10")], "10");
  timeline.applyEvent(reaction("11"));
  assert.equal(timeline.messages.length, 1);
  timeline.prepend([message("1")]);
  assert.deepEqual(timeline.messages[0].reactions, reaction("11").reactions);
  assert.equal(timeline.cursor, "11");
  timeline.reset([{ ...message("1"), reactionSeq: "12", reactions: [] }], "12");
  timeline.applyEvent(reaction("11"));
  assert.deepEqual(timeline.messages[0].reactions, []);
});

test("reaction validators reject malformed revisions and duplicate actor counts", () => {
  assert.ok(isChatReactionEvent(reaction("9007199254740993")));
  for (const invalid of [
    { ...reaction("1"), seq: 1 }, { ...reaction("1"), schemaVersion: 2 },
    reaction("-1"), reaction("1", ["guest", "guest"]),
    { ...reaction("1"), reactions: [{ emoji: "👍", authorIds: [] }] },
  ]) assert.equal(isChatReactionEvent(invalid), false);
});

test("Twemoji filenames handle selectors, keycaps, flags, skin tones and ZWJ sequences", () => {
  for (const [emoji, filename] of [["❤️", "2764"], ["1️⃣", "31-20e3"], ["🇩🇴", "1f1e9-1f1f4"], ["👍🏽", "1f44d-1f3fd"], ["👩‍⚕️", "1f469-200d-2695-fe0f"]]) {
    assert.equal(emojiAsset(emojiCode(emoji)), `/emoji/twemoji-15/${filename}.svg`);
  }
  assert.equal(emojiAsset("0031-fe0f-20e3"), "/emoji/twemoji-15/31-20e3.svg");
});
