import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { emojiLabel, emojiNameFrom, emojiNameIndex, fallbackSummary, isReactionList, reactionSummary, reactorName } from "../chat/reactors.ts";

const people = (...names: string[]) => names.map((name) => ({ id: name.toLowerCase(), name }));

test("who-reacted summaries match the wording shared with native clients", () => {
  const label = ":thumbs-up:";
  assert.equal(reactionSummary(people("Me"), "me", label), "You reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Bob"), "me", label), "Bob reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Bob", "Me"), "me", label), "You and Bob reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Alice", "Bob", "Carol"), "me", ":party-popper:"), "Alice, Bob and Carol reacted with :party-popper:");
  assert.equal(reactionSummary(people("Alice", "Bob", "Carol", "Dan"), undefined, label), "Alice, Bob, Carol and 1 other reacted with :thumbs-up:");
  assert.equal(reactionSummary(people("Alice", "Bob", "Me", "Carol", "Dan"), "me", label), "You, Alice, Bob and 2 others reacted with :thumbs-up:");
  assert.equal(reactionSummary([], "me", "👍"), "Someone reacted with 👍");
});

test("snapshot fallbacks count people until names load", () => {
  assert.equal(fallbackSummary({ emoji: "👍", authorIds: ["me"] }, "me", ":thumbs-up:"), "You reacted with :thumbs-up:");
  assert.equal(fallbackSummary({ emoji: "👍", authorIds: ["bob"] }, "me", ":thumbs-up:"), "1 person reacted with :thumbs-up:");
  assert.equal(fallbackSummary({ emoji: "👍", authorIds: ["me", "bob", "carol"] }, "me", ":thumbs-up:"), "3 people reacted with :thumbs-up:");
});

test("names, labels and responses are validated with safe fallbacks", () => {
  assert.equal(reactorName({ displayName: "Bob B", username: "bob" }), "Bob B");
  assert.equal(reactorName({ displayName: null, username: "bob" }), "bob");
  assert.equal(reactorName({ displayName: "", username: null }), "Someone");
  assert.equal(emojiLabel("👍", "thumbs-up"), ":thumbs-up:");
  assert.equal(emojiLabel("👍"), "👍");
  const valid = { messageId: "m1", reactionSeq: "12", reactions: [{ emoji: "👍", authors: [{ id: "bob", username: "bob", displayName: "Bob B", avatarId: 101 }, { id: "x", username: null, displayName: null, avatarId: null }] }] };
  assert.equal(isReactionList(valid), true);
  assert.equal(isReactionList({ ...valid, reactionSeq: "01" }), false);
  assert.equal(isReactionList({ ...valid, reactions: [{ emoji: "👍", authors: [{ id: 3 }] }] }), false);
  assert.equal(isReactionList(null), false);
});

test("emoji names come from the picker data and ignore variation selectors", () => {
  const require = createRequire(import.meta.url);
  const english = JSON.parse(readFileSync(require.resolve("emoji-picker-react/dist/data/emojis-en.json"), "utf8"));
  const index = emojiNameIndex(english.emojis);
  for (const [emoji, name] of [["👍", "thumbs-up"], ["😂", "face-with-tears-of-joy"], ["🎉", "party-popper"], ["👀", "looking"], ["❤️", "red-heart"], ["❤", "red-heart"]]) {
    assert.equal(emojiNameFrom(index, emoji), name, emoji);
  }
  for (const [emoji, label] of [["🇮🇱", ":israel:"], ["🇺🇸", ":united-states:"], ["🇨🇮", ":cote-divoire:"]]) {
    const name = emojiNameFrom(index, emoji);
    assert.equal(emojiLabel(emoji, name), label);
    assert.equal(fallbackSummary({ emoji, authorIds: ["me"] }, "me", emojiLabel(emoji, name)), `You reacted with ${label}`);
  }
});
