import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test, vi } from "vitest";
import {
  cachedReactors,
  emojiLabel,
  emojiNameFrom,
  emojiNameIndex,
  fallbackSummary,
  isReactionList,
  loadReactors,
  reactionPeople,
  reactionSummary,
  reactorName,
  type ReactionList,
} from "../chat/reactors.ts";

const people = (...names: string[]) => names.map((name) => ({ id: name.toLowerCase(), name }));

test("cached names follow displayed membership through optimistic toggles and newer revisions", () => {
  const list: ReactionList = {
    messageId: "optimistic",
    reactionSeq: "1",
    reactions: [
      {
        emoji: "👍",
        authors: ["Bob", "Alice"].map((name) => ({
          id: name.toLowerCase(),
          displayName: name,
          username: null,
          avatarId: null,
        })),
      },
      { emoji: "❤️", authors: [{ id: "carol", displayName: "Carol", username: null, avatarId: null }] },
    ],
  };
  const resolve = (...authorIds: string[]) => reactionPeople({ emoji: "👍", authorIds }, "me", list);
  assert.deepEqual(resolve("alice", "bob", "me"), [...people("Bob", "Alice"), { id: "me", name: "You" }]);
  assert.equal(reactionSummary(resolve("bob", "me")!, "me", ":thumbs-up:"), "You and Bob reacted with :thumbs-up:");
  assert.deepEqual(resolve("alice"), people("Alice"), "removed IDs never survive in the summary");
  assert.deepEqual(resolve("bob", "carol"), people("Bob", "Carol"), "other emoji can supply known names");
  assert.equal(resolve("bob", "unknown"), undefined, "unknown IDs require the count fallback");
  assert.deepEqual(reactionPeople({ emoji: "👍", authorIds: ["me"] }, "me"), [{ id: "me", name: "You" }]);
  assert.equal(reactionPeople({ emoji: "👍", authorIds: ["bob"] }, "me"), undefined);
});

test("slower old reactor requests cannot evict a newer cached revision and trigger another fetch", async () => {
  const responses: Array<(response: Response) => void> = [];
  const fetch = vi.fn(() => new Promise<Response>((resolve) => responses.push(resolve)));
  vi.stubGlobal("fetch", fetch);
  const newer: ReactionList = {
    messageId: "cache-race",
    reactionSeq: "9007199254740993",
    reactions: [{ emoji: "👍", authors: [{ id: "bob", username: "bob", displayName: "Bob", avatarId: 101 }] }],
  };
  const older: ReactionList = { ...newer, reactionSeq: "9007199254740992", reactions: [] };
  const oldRequest = loadReactors("channel", newer.messageId, older.reactionSeq);
  const newRequest = loadReactors("channel", newer.messageId, newer.reactionSeq);
  assert.equal(loadReactors("channel", newer.messageId, newer.reactionSeq), newRequest, "in-flight reads coalesce");
  responses[1](Response.json(newer));
  await newRequest;
  responses[0](Response.json(older));
  await oldRequest;
  assert.deepEqual(cachedReactors("channel", newer.messageId, newer.reactionSeq), newer);
  assert.equal(cachedReactors("channel", newer.messageId, older.reactionSeq), undefined);
  assert.deepEqual(await loadReactors("channel", newer.messageId, newer.reactionSeq), newer);
  assert.equal(fetch.mock.calls.length, 2, "the current revision stays reusable after out-of-order responses");
});

test("failed reactor requests are retryable and caches remain scoped to the channel", async () => {
  const list: ReactionList = { messageId: "retry-cache", reactionSeq: "7", reactions: [] };
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(Response.json({ error: "Try again" }, { status: 503 }))
    .mockResolvedValueOnce(Response.json(list))
    .mockResolvedValueOnce(Response.json({ ...list, reactionSeq: "8" }));
  vi.stubGlobal("fetch", fetch);
  await assert.rejects(loadReactors("channel-a", list.messageId, "7"), /Try again/);
  assert.deepEqual(await loadReactors("channel-a", list.messageId, "7"), list);
  assert.equal(cachedReactors("channel-b", list.messageId, "7"), undefined);
  await loadReactors("channel-b", list.messageId, "8");
  assert.deepEqual(cachedReactors("channel-a", list.messageId, "7"), list);
  assert.equal(fetch.mock.calls.length, 3);
});

test("who-reacted summaries match the wording shared with native clients", () => {
  const label = ":thumbs-up:";
  assert.equal(reactionSummary(people("Me"), "me", label), "You reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Bob"), "me", label), "Bob reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Bob", "Me"), "me", label), "You and Bob reacted with :thumbs-up:");
  assert.equal(
    reactionSummary(people("Alice", "Bob", "Carol"), "me", ":party-popper:"),
    "Alice, Bob and Carol reacted with :party-popper:",
  );
  assert.equal(
    reactionSummary(people("Alice", "Bob", "Carol", "Dan"), undefined, label),
    "Alice, Bob, Carol and 1 other reacted with :thumbs-up:",
  );
  assert.equal(
    reactionSummary(people("Alice", "Bob", "Me", "Carol", "Dan"), "me", label),
    "You, Alice, Bob and 2 others reacted with :thumbs-up:",
  );
  assert.equal(reactionSummary([], "me", "👍"), "Someone reacted with 👍");
});

test("snapshot fallbacks count people until names load", () => {
  assert.equal(
    fallbackSummary({ emoji: "👍", authorIds: ["me"] }, "me", ":thumbs-up:"),
    "You reacted with :thumbs-up:",
  );
  assert.equal(
    fallbackSummary({ emoji: "👍", authorIds: ["bob"] }, "me", ":thumbs-up:"),
    "1 person reacted with :thumbs-up:",
  );
  assert.equal(
    fallbackSummary({ emoji: "👍", authorIds: ["me", "bob", "carol"] }, "me", ":thumbs-up:"),
    "3 people reacted with :thumbs-up:",
  );
});

test("names, labels and responses are validated with safe fallbacks", () => {
  assert.equal(reactorName({ displayName: "Bob B", username: "bob" }), "Bob B");
  assert.equal(reactorName({ displayName: null, username: "bob" }), "bob");
  assert.equal(reactorName({ displayName: "", username: null }), "Someone");
  assert.equal(emojiLabel("👍", "thumbs-up"), ":thumbs-up:");
  assert.equal(emojiLabel("👍"), "👍");
  const valid = {
    messageId: "m1",
    reactionSeq: "12",
    reactions: [
      {
        emoji: "👍",
        authors: [
          { id: "bob", username: "bob", displayName: "Bob B", avatarId: 101 },
          { id: "x", username: null, displayName: null, avatarId: null },
        ],
      },
    ],
  };
  assert.equal(isReactionList(valid), true);
  assert.equal(isReactionList({ ...valid, reactionSeq: "01" }), false);
  assert.equal(isReactionList({ ...valid, reactions: [{ emoji: "👍", authors: [{ id: 3 }] }] }), false);
  assert.equal(isReactionList(null), false);
});

test("emoji names come from the picker data and ignore variation selectors", () => {
  const require = createRequire(import.meta.url);
  const english = JSON.parse(readFileSync(require.resolve("emoji-picker-react/dist/data/emojis-en.json"), "utf8"));
  const index = emojiNameIndex(english.emojis);
  for (const [emoji, name] of [
    ["👍", "thumbs-up"],
    ["😂", "face-with-tears-of-joy"],
    ["🎉", "party-popper"],
    ["👀", "looking"],
    ["❤️", "red-heart"],
    ["❤", "red-heart"],
  ]) {
    assert.equal(emojiNameFrom(index, emoji), name, emoji);
  }
  for (const [emoji, label] of [
    ["🇮🇱", ":israel:"],
    ["🇺🇸", ":united-states:"],
    ["🇨🇮", ":cote-divoire:"],
  ]) {
    const name = emojiNameFrom(index, emoji);
    assert.equal(emojiLabel(emoji, name), label);
    assert.equal(
      fallbackSummary({ emoji, authorIds: ["me"] }, "me", emojiLabel(emoji, name)),
      `You reacted with ${label}`,
    );
  }
});
