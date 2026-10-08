import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test, vi } from "vitest";
import { ChatTimeline } from "../chat/timeline.ts";
import {
  isChatMessage,
  isChannelMessage,
  isChatPinEvent,
  isChatReactionEvent,
  type ChatMessage,
  type ChatPinEvent,
  type ChatReactionEvent,
} from "../chat/types.ts";
import { emojiAsset, emojiCode, emojiNames, preloadEmojiImages } from "../chat/emoji.ts";

function message(seq: string, id = `message-${seq}`): ChatMessage {
  return {
    id,
    channelId: "general",
    seq,
    author: { id: "guest", name: "Guest", isGuest: true },
    content: { version: 1, type: "text", text: `<b>safe ${seq}</b>` },
    createdAt: "2026-09-21T12:00:00Z",
    clientMessageId: `client-${seq}`,
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
  assert.deepEqual(
    timeline.messages.map((value) => value.seq),
    ["9007199254740992", "9007199254740993", "9007199254740994"],
  );
});

test("an HTTP send merges immediately but only its matching event advances replay", () => {
  const timeline = new ChatTimeline();
  timeline.reset([], "5");
  const sent = message("6", "durable-send");

  timeline.mergeSent(sent);
  assert.equal(timeline.cursor, "5");
  assert.deepEqual(
    timeline.messages.map((value) => value.id),
    ["durable-send"],
  );

  assert.equal(timeline.applyEvent(sent), "applied");
  assert.equal(timeline.cursor, "6");
  assert.equal(timeline.messages.length, 1, "the event deduplicates the HTTP response by message id");
});

function reaction(seq: string, authorIds = ["other"], messageId = "message-1"): ChatReactionEvent {
  return {
    type: "message.reactions",
    schemaVersion: 1,
    channelId: "general",
    messageId,
    seq,
    reactions: authorIds.length ? [{ emoji: "👍🏽", authorIds }] : [],
  };
}

function pin(seq: string, active = true, target = message("1")): ChatPinEvent {
  const pinned = {
    ...target,
    pinSeq: seq,
    pin: active
      ? { author: { id: "moderator", name: "Mod", isGuest: false }, createdAt: "2026-09-21T13:00:00Z" }
      : null,
  };
  return { type: "message.pin", schemaVersion: 1, channelId: "general", seq, message: pinned };
}

test("pins interleave with messages and reactions without changing creation order", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1", []);
  assert.equal(timeline.applyEvent(pin("2")), "applied");
  assert.equal(timeline.applyEvent(reaction("3")), "applied");
  assert.equal(timeline.applyEvent(message("4")), "applied");
  assert.equal(timeline.cursor, "4");
  assert.deepEqual(
    timeline.messages.map((item) => item.seq),
    ["1", "4"],
  );
  assert.deepEqual(
    timeline.pinnedMessages.map((item) => item.id),
    ["message-1"],
  );
  assert.ok(isChatPinEvent(pin("9007199254740993")));
});

test("pinned reaction snapshots stay current across replay, pin acknowledgements and stale pages", () => {
  for (const loaded of [false, true]) {
    const timeline = new ChatTimeline();
    const original = pin("2").message;
    timeline.reset(loaded ? [original, message("10")] : [message("10")], "10", [original]);
    timeline.mergeReactions(reaction("12", ["alice", "bob"]));
    assert.equal(timeline.cursor, "10", "HTTP confirmation does not advance replay");
    assert.deepEqual(timeline.pinnedMessages[0].reactions, [{ emoji: "👍🏽", authorIds: ["alice", "bob"] }]);
    assert.equal(timeline.applyEvent(reaction("11", ["alice"])), "applied");
    timeline.mergePin(pin("13", true, original));
    assert.equal(timeline.pinnedMessages[0].reactionSeq, "12", "a later pin has an independent revision");
    timeline.prepend([original]);
    assert.equal(timeline.messages[0].reactionSeq, "12", "old pages cannot overwrite the reaction");
    timeline.mergeReactions(reaction("14", []));
    timeline.mergePin(pin("13", true, original));
    assert.deepEqual(timeline.pinnedMessages[0].reactions, [], "a delayed pin must not resurrect reactions");
    assert.equal(timeline.pinnedMessages[0].reactionSeq, "14");
    assert.equal(timeline.messages.length, 2, "pins never insert rows into channel pagination");
    timeline.mergePin(
      pin("2", true, { ...original, reactionSeq: "15", reactions: reaction("15", ["carol"]).reactions }),
    );
    assert.equal(timeline.pinnedMessages[0].pinSeq, "13", "old pin metadata stays rejected");
    assert.deepEqual(timeline.pinnedMessages[0].reactions, [{ emoji: "👍🏽", authorIds: ["carol"] }]);
    assert.equal(timeline.messages[0].reactionSeq, "15", "a no-op pin can still carry newer reactions");
  }
});

test("an old pin outside the loaded page updates live and stale snapshots cannot resurrect its unpin", () => {
  const timeline = new ChatTimeline();
  const old = message("1", "old-pin");
  timeline.reset([message("10")], "10", [{ ...old, ...pin("8", true, old).message }]);
  assert.deepEqual(
    timeline.messages.map((item) => item.id),
    ["message-10"],
    "pins do not enter timeline pagination",
  );
  timeline.applyEvent(pin("11", false, old));
  assert.equal(timeline.pinnedMessages.length, 0);
  timeline.prepend([{ ...old, ...pin("8", true, old).message }]);
  assert.equal(timeline.messages[0].pin, null, "the newer unpin tombstone wins on the old page");
  timeline.reset([message("10")], "11", [pin("8", true, old).message]);
  assert.equal(timeline.pinnedMessages.length, 0, "a stale history snapshot cannot resurrect the pin");
});

test("authoritative reconnect removes offline unpins and independent pin revisions survive stale pages", () => {
  const timeline = new ChatTimeline();
  const old = pin("8").message;
  timeline.reset([message("10")], "10", [old]);
  timeline.applyEvent(pin("11"));
  timeline.reset([message("14")], "14", []);
  assert.equal(
    timeline.pinnedMessages.length,
    0,
    "absence at a newer history cursor removes a pin changed while offline",
  );
  timeline.reset([old], "8", [old]);
  timeline.prepend([{ ...pin("12", false).message, reactionSeq: "5", reactions: [] }]);
  timeline.prepend([{ ...old, reactionSeq: "13", reactions: reaction("13").reactions }]);
  assert.equal(timeline.messages[0].pin, null);
  assert.equal(timeline.messages[0].reactionSeq, "13", "reaction and pin revisions merge independently");
  assert.equal(timeline.pinnedMessages.length, 0);
  timeline.reset([], "0");
  assert.equal(timeline.pinnedMessages.length, 0, "access revocation clears every pin snapshot");
});

test("complete pin history rejects old acknowledgements and pages but preserves newer HTTP snapshots", () => {
  const timeline = new ChatTimeline();
  const old = message("1", "unloaded-pin");
  timeline.reset([message("50")], "60", []);
  timeline.mergePin(pin("4", true, old));
  timeline.mergePin(pin("60", true, old));
  assert.equal(
    timeline.pinnedMessages.length,
    0,
    "absence from complete history supersedes acknowledgements through its cursor",
  );
  timeline.prepend([pin("4", true, old).message]);
  assert.equal(timeline.messages[0].pin, null, "a stale page must not restore the inline marker either");
  timeline.mergePin(pin("61", true, old));
  assert.equal(timeline.cursor, "60", "HTTP must not advance replay");
  timeline.reset([message("50")], "60", []);
  assert.deepEqual(
    timeline.pinnedMessages.map((item) => item.id),
    [old.id],
    "a newer acknowledgement survives a concurrently captured history",
  );
  assert.equal(timeline.applyEvent(pin("61", true, old)), "applied");
  assert.equal(timeline.cursor, "61");
});

test("reactions fill sequence gaps without becoming messages or accepting stale acknowledgements", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("1")], "1");
  assert.equal(timeline.applyEvent(message("3")), "buffered");
  timeline.mergeReactions(reaction("4", ["other", "guest"]));
  assert.equal(timeline.cursor, "1", "HTTP reactions cannot skip missing events");
  assert.equal(timeline.applyEvent(reaction("2")), "applied");
  assert.equal(timeline.cursor, "3");
  assert.deepEqual(
    timeline.messages.map((m) => m.seq),
    ["1", "3"],
  );
  assert.deepEqual(timeline.messages[0].reactions, reaction("4", ["other", "guest"]).reactions);
  timeline.applyEvent(reaction("4", ["other", "guest"]));
  timeline.applyEvent(reaction("5", []));
  timeline.mergeReactions(reaction("4", ["other", "guest"]));
  timeline.prepend([{ ...message("1"), reactions: reaction("2").reactions, reactionSeq: "2" }]);
  assert.deepEqual(
    timeline.messages[0].reactions,
    [],
    "late HTTP and paginated snapshots cannot resurrect a removed reaction",
  );
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
    { ...reaction("1"), seq: 1 },
    { ...reaction("1"), schemaVersion: 2 },
    reaction("-1"),
    reaction("1", ["guest", "guest"]),
    { ...reaction("1"), reactions: [{ emoji: "👍", authorIds: [] }] },
  ])
    assert.equal(isChatReactionEvent(invalid), false);
});

test("Twemoji filenames handle selectors, keycaps, flags, skin tones and ZWJ sequences", () => {
  for (const [emoji, filename] of [
    ["❤️", "2764"],
    ["1️⃣", "31-20e3"],
    ["🇩🇴", "1f1e9-1f1f4"],
    ["👍🏽", "1f44d-1f3fd"],
    ["👩‍⚕️", "1f469-200d-2695-fe0f"],
  ]) {
    assert.equal(emojiAsset(emojiCode(emoji)), `/emoji/twemoji-15/${filename}.svg`);
  }
  assert.equal(emojiAsset("0031-fe0f-20e3"), "/emoji/twemoji-15/31-20e3.svg");
});

test("emoji names prefer dashes while retaining spaced and underscore search aliases", () => {
  const original = ["happy_face", "grinning face"];
  assert.deepEqual(emojiNames(original), [
    "happy_face",
    "happy face",
    "happy-face",
    "grinning face",
    "grinning_face",
    "grinning-face",
  ]);
  assert.deepEqual(original, ["happy_face", "grinning face"], "the package catalog must stay unchanged");
  assert.deepEqual(emojiNames(["thumbs-up", "+1", "thumbs up"]), ["thumbs up", "thumbs_up", "+1", "thumbs-up"]);
  assert.equal(emojiNames(["face_with  big_eyes"]).at(-1), "face-with-big-eyes");
});

test("country flags prefer typeable country names and retain their original aliases", () => {
  for (const [code, label, expected] of [
    ["IL", "Israel", "israel"],
    ["US", "United States", "united-states"],
    ["BA", "Bosnia & Herzegovina", "bosnia-and-herzegovina"],
    ["CI", "Côte d’Ivoire", "cote-divoire"],
    ["UM", "U.S. Outlying Islands", "us-outlying-islands"],
    ["MM", "Myanmar (Burma)", "myanmar-burma"],
    ["TR", "Türkiye", "turkiye"],
    ["GB", "England", "england"],
  ]) {
    const original = [code, "flag", `flag: ${label}`];
    const aliases = emojiNames(original);
    assert.equal(aliases.at(-1), expected);
    for (const alias of [
      code,
      "flag",
      `flag: ${label}`,
      expected.replaceAll("-", "_"),
      expected.replaceAll("-", " "),
    ]) {
      assert.ok(aliases.includes(alias), alias);
    }
    assert.deepEqual(original, [code, "flag", `flag: ${label}`]);
  }
  for (const label of ["rainbow flag", "pirate flag", "chequered flag"]) {
    assert.equal(emojiNames(["flag", label]).at(-1), label.replaceAll(" ", "-"));
  }
});

test("bundled native emoji names and aliases match on Android, Apple and desktop", () => {
  const shared = readFileSync(new URL("../../../../shared/emoji/catalog.json", import.meta.url), "utf8");
  const apple = readFileSync(
    new URL("../../../native/apple/Sources/CaperCore/EmojiAssets/catalog.json", import.meta.url),
    "utf8",
  );
  assert.equal(apple, shared);
  const entries: { id: string; name: string; keywords: string; selectable: boolean; emoji: string }[] =
    JSON.parse(shared);
  assert.ok(entries.filter((entry) => entry.selectable).every((entry) => !/[\s_]/.test(entry.name)));
  const grinning = entries.find((entry) => entry.id === "1f600")!;
  assert.equal(grinning.name, "grinning-face");
  assert.equal(grinning.emoji, "😀");
  for (const query of ["grinning-face", "grinning_face", "grinning face"]) assert.ok(grinning.keywords.includes(query));
  const flags = entries.filter((entry) => entry.selectable && entry.keywords.includes("flag:"));
  assert.ok(flags.length > 250, "country and regional flags must be covered");
  assert.ok(
    flags.every((entry) => /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(entry.name)),
    "every country name must be typeable in colon autocomplete",
  );
  for (const [id, emoji, name] of [
    ["1f1ee-1f1f1", "🇮🇱", "israel"],
    ["1f1fa-1f1f8", "🇺🇸", "united-states"],
    ["1f1e8-1f1ee", "🇨🇮", "cote-divoire"],
  ]) {
    const flag = flags.find((entry) => entry.id === id)!;
    assert.equal(flag.name, name);
    assert.equal(flag.emoji, emoji);
    assert.ok(flag.keywords.includes(name));
  }
});

test("emoji preload shares decoding per category, retries failures, and stays warm across messages", async (t) => {
  const manifest = ["1f600", "0031-fe0f-20e3", "1f469-200d-2695-fe0f"];
  const fetchMock = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (url) =>
      Response.json(String(url).endsWith("preload-flags.json") ? ["1f3c1", "1f1e9-1f1f4"] : manifest),
    );
  fetchMock.mockImplementationOnce(async () => new Response(null, { status: 503 }));
  const images: { src: string }[] = [];
  const decoded: (() => void)[] = [];
  let failImage = true;
  const originalImage = globalThis.Image;
  globalThis.Image = class {
    src = "";
    constructor() {
      images.push(this);
    }
    decode() {
      return failImage
        ? Promise.reject(new Error("Artwork unavailable"))
        : new Promise<void>((resolve) => decoded.push(resolve));
    }
  } as unknown as typeof Image;
  t.onTestFinished(() => {
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
  assert.equal(fetchMock.mock.calls.length, 3, "manifest and image failures must both allow retry");
  assert.deepEqual(
    images.slice(3).map((image) => image.src),
    ["/emoji/twemoji-15/1f600.svg", "/emoji/twemoji-15/31-20e3.svg", "/emoji/twemoji-15/1f469-200d-2695-fe0f.svg"],
  );
  assert.equal(decoded.length, 3);
  decoded.forEach((finish) => finish());
  await first;
  assert.strictEqual(preloadEmojiImages(), first, "a later message must reuse completed preload work");
  assert.equal(fetchMock.mock.calls.length, 3);
  assert.ok(fetchMock.mock.calls.every(([url]) => url === "/emoji/twemoji-15/preload.json"));

  failImage = true;
  await preloadEmojiImages("flags");
  assert.strictEqual(preloadEmojiImages(), first, "a category failure must not evict another category");
  failImage = false;
  const flags = preloadEmojiImages("flags");
  const food = preloadEmojiImages("food_drink");
  assert.strictEqual(preloadEmojiImages("flags"), flags, "concurrent intent must share only its category");
  assert.notStrictEqual(food, flags, "different categories must preload independently");
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(
    fetchMock.mock.calls.slice(3).map(([url]) => url),
    [
      "/emoji/twemoji-15/preload-flags.json",
      "/emoji/twemoji-15/preload-flags.json",
      "/emoji/twemoji-15/preload-food_drink.json",
    ],
  );
  assert.deepEqual(
    images.slice(8, 10).map((image) => image.src),
    ["/emoji/twemoji-15/1f3c1.svg", "/emoji/twemoji-15/1f1e9-1f1f4.svg"],
  );
  decoded.slice(3).forEach((finish) => finish());
  await Promise.all([flags, food]);
  assert.strictEqual(preloadEmojiImages("flags"), flags);
  assert.strictEqual(preloadEmojiImages("food_drink"), food);
  assert.equal(fetchMock.mock.calls.length, 6, "completed categories must stay warm when the picker reopens");
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
  assert.deepEqual(
    drained.map(({ seq }) => seq),
    ["1", "2", "3", "4", "5"],
  );

  timeline.reset([], "9");
  assert.notStrictEqual(timeline.messages, drained);
  assert.deepEqual(timeline.messages, []);
});

test("cached snapshots preserve BigInt ordering, id tie breaks, and immutability", () => {
  const timeline = new ChatTimeline();
  timeline.reset(
    [message("9007199254740993", "z"), message("9007199254740992", "middle"), message("9007199254740993", "a")],
    "9007199254740993",
  );
  const snapshot = timeline.messages;

  assert.deepEqual(
    snapshot.map(({ id }) => id),
    ["middle", "a", "z"],
  );
  timeline.mergeSent(message("9007199254740994", "later"));
  assert.deepEqual(
    snapshot.map(({ id }) => id),
    ["middle", "a", "z"],
    "previous snapshots must not mutate",
  );
  assert.deepEqual(
    timeline.messages.map(({ id }) => id),
    ["middle", "a", "z", "later"],
  );
});

test("sorted snapshots are reused until visible messages change and never mutate previous snapshots", () => {
  const timeline = new ChatTimeline();
  timeline.reset([message("12"), message("10")], "12");
  const initial = timeline.messages;
  assert.deepEqual(
    initial.map((value) => value.seq),
    ["10", "12"],
  );
  assert.equal(timeline.messages, initial, "reading must not repeatedly allocate and sort history");
  timeline.applyEvent(message("12"));
  timeline.applyEvent(message("14"));
  assert.equal(timeline.messages, initial, "duplicates and buffered events leave the visible snapshot intact");
  timeline.prepend([message("9")]);
  const paginated = timeline.messages;
  assert.deepEqual(
    paginated.map((value) => value.seq),
    ["9", "10", "12"],
  );
  timeline.mergeSent(message("15"));
  assert.deepEqual(
    timeline.messages.map((value) => value.seq),
    ["9", "10", "12", "15"],
  );
  timeline.applyEvent(message("13"));
  assert.deepEqual(
    timeline.messages.map((value) => value.seq),
    ["9", "10", "12", "13", "14", "15"],
  );
  assert.deepEqual(
    initial.map((value) => value.seq),
    ["10", "12"],
  );
  assert.deepEqual(
    paginated.map((value) => value.seq),
    ["9", "10", "12"],
  );
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
  const fresh = {
    ...message("1"),
    author: { ...message("1").author, name: "Fresh name" },
    reactions: [],
    reactionSeq: "2",
  };
  const cached = { ...message("1"), reactions: reaction("3").reactions, reactionSeq: "3" };
  timeline.reset([fresh, cached], "2");
  assert.equal(timeline.messages[0].author.name, "Fresh name");
  assert.deepEqual(timeline.messages[0].reactions, reaction("3").reactions);
  assert.equal(timeline.cursor, "2");
});

test("thread summaries survive stale history, unloaded parents and duplicate broadcast replies", () => {
  const timeline = new ChatTimeline();
  const first = { replyCount: 1, participants: [message("1").author], seq: "12" };
  const latest = { replyCount: 3, participants: [{ id: "other", name: "Other", isGuest: false }], seq: "15" };
  timeline.reset([message("10")], "10");
  const reply = { ...message("15"), threadRootId: "parent", broadcast: true, thread: latest };
  timeline.mergeSent(reply);
  assert.equal(timeline.cursor, "10");
  timeline.prepend([{ ...message("1", "parent"), thread: first }]);
  assert.deepEqual(timeline.messages[0].thread, latest, "a late parent page cannot lower the summary revision");
  timeline.prepend([{ ...reply, thread: first, broadcast: true }]);
  assert.equal(timeline.messages.filter((row) => row.id === reply.id).length, 1);
  assert.deepEqual(timeline.messages[0].thread, latest);
  assert.equal(isChannelMessage(reply), true);
  assert.equal(isChannelMessage({ ...reply, broadcast: false }), false);
  assert.equal(timeline.cursor, "10", "thread GET/HTTP ACK never advances channel replay");
});

test("thread metadata validators reject invalid roots, broadcasts and summary revisions", () => {
  const root = message("1");
  assert.ok(isChatMessage({ ...root, threadRootId: "root", broadcast: false }));
  for (const invalid of [
    { ...root, broadcast: true },
    { ...root, threadRootId: "" },
    { ...root, thread: { replyCount: 1, participants: [root.author], seq: "-1" } },
    { ...root, thread: { replyCount: 0, participants: [root.author], seq: "2" } },
    { ...root, thread: { replyCount: 2, participants: [root.author, root.author], seq: "2" } },
  ])
    assert.equal(isChatMessage(invalid), false);
});
