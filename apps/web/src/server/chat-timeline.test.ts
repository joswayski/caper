import assert from "node:assert/strict";
import test from "node:test";
import { ChatTimeline } from "../chat/timeline.ts";
import { isChatReactionEvent, type ChatMessage, type ChatReactionEvent } from "../chat/types.ts";
import { emojiAsset, emojiCode, preloadEmojiImages } from "../chat/emoji.ts";

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

test("emoji preload shares image decoding, retries failures, and stays warm across messages", async (t) => {
  const manifest = ["1f600", "0031-fe0f-20e3", "1f469-200d-2695-fe0f"];
  const fetchMock = t.mock.method(globalThis, "fetch", async () => Response.json(manifest));
  fetchMock.mock.mockImplementationOnce(async () => new Response(null, { status: 503 }));
  const images: { src: string }[] = [];
  const decoded: (() => void)[] = [];
  let failImage = true;
  const originalImage = globalThis.Image;
  globalThis.Image = class {
    src = "";
    constructor() { images.push(this); }
    decode() {
      return failImage ? Promise.reject(new Error("Artwork unavailable")) : new Promise<void>((resolve) => decoded.push(resolve));
    }
  } as unknown as typeof Image;
  t.after(() => {
    if (originalImage) globalThis.Image = originalImage;
    else Reflect.deleteProperty(globalThis, "Image");
  });

  await preloadEmojiImages();
  assert.equal(images.length, 0, "a failed manifest must not start image requests");
  await preloadEmojiImages();
  assert.equal(images.length, 3);
  failImage = false;
  const first = preloadEmojiImages();
  assert.strictEqual(preloadEmojiImages(), first, "simultaneous hovers must share the preload");
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(fetchMock.mock.callCount(), 3, "manifest and image failures must both allow retry");
  assert.deepEqual(images.slice(3).map((image) => image.src), [
    "/emoji/twemoji-15/1f600.svg", "/emoji/twemoji-15/31-20e3.svg", "/emoji/twemoji-15/1f469-200d-2695-fe0f.svg",
  ]);
  assert.equal(decoded.length, 3);
  decoded.forEach((finish) => finish());
  await first;
  assert.strictEqual(preloadEmojiImages(), first, "a later message must reuse completed preload work");
  assert.equal(fetchMock.mock.callCount(), 3);
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

test("reaction snapshots invalidate cached visible arrays without mutating old snapshots", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1");
  const before = timeline.messages;
  timeline.mergeReactions(reaction("3"));
  const after = timeline.messages;
  assert.notStrictEqual(after, before);
  assert.equal(before[0].reactions, undefined);
  assert.deepEqual(after[0].reactions, reaction("3").reactions);
  timeline.applyEvent(reaction("2"));
  assert.strictEqual(timeline.messages, after, "older reaction events leave the newest snapshot cached");
  assert.equal(timeline.cursor, "2", "snapshot caching must not advance replay from HTTP");
  timeline.applyEvent(reaction("3"));
  assert.strictEqual(timeline.messages, after);
});

test("fresh author metadata coexists with newer cached reaction revisions", () => {
  const timeline = new ChatTimeline();
  const fresh = { ...message("1"), author: { ...message("1").author, name: "Fresh name" }, reactions: [], reactionSeq: "2" };
  const cached = { ...message("1"), reactions: reaction("3").reactions, reactionSeq: "3" };
  timeline.reset([fresh, cached], "2");
  assert.equal(timeline.messages[0].author.name, "Fresh name");
  assert.deepEqual(timeline.messages[0].reactions, reaction("3").reactions);
  assert.equal(timeline.cursor, "2");
});
