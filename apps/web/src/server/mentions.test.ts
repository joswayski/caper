import assert from "node:assert/strict";
import { test } from "node:test";
import { insertMention, mentionCardPerson, mentionName, mentionSegments, mentionSuggestions, mentionToken, mentionsAccount, type MentionCandidate } from "../chat/mentions.ts";
import type { ChatMessage } from "../chat/types.ts";

const members: MentionCandidate[] = [
  { id: "u-maya", username: "maya", displayName: "Maya Lopez" },
  { id: "u-mo", username: "mo", displayName: "Mo" },
  { id: "u-sam", username: "sam_m", displayName: "Samuel Maya" },
  { id: "u-al", username: "alfie", displayName: "Alfred" },
  { id: "u-zz", username: "zz_maya", displayName: "Zed" },
  { id: "u-b", username: "bea", displayName: "Bea" },
  { id: "u-c", username: "cal", displayName: "Cal" },
];

test("@ autocomplete uses the shared start rule and caret guards", () => {
  for (const text of ["@", "@ma", "hi @ma", "(@ma", "[@ma", "{@ma", "line\n@ma", "@Maya_1"]) {
    assert.ok(mentionToken(text, text.length), text);
  }
  for (const text of ["bob@ma", "x/@ma", "@@ma", "a,@ma", `@${"a".repeat(33)}`]) {
    assert.equal(mentionToken(text, text.length), undefined, text);
  }
  assert.equal(mentionToken("@maya", 3), undefined, "caret inside a name");
  assert.equal(mentionToken("@ma", 0, 3), undefined, "selection");
  assert.equal(mentionToken("@ma@", 3), undefined, "followed by @");
  assert.deepEqual(mentionToken("ok @ma suffix", 6), { start: 3, end: 6, query: "ma" });
  assert.deepEqual(mentionToken("🙂 @ma", 6), { start: 3, end: 6, query: "ma" }, "UTF-16 offsets");
});

test("suggestions rank username, display name and contains; specials keep their slots", () => {
  const names = (query: string, specials = true) => mentionSuggestions(members, query, specials).map(mentionName);
  assert.deepEqual(names("maya"), ["maya", "sam_m", "zz_maya"]);
  assert.deepEqual(names("MA"), ["maya", "sam_m", "zz_maya"]);
  assert.deepEqual(names("e"), ["alfie", "bea", "everyone"]);
  assert.deepEqual(names("h"), ["here"]);
  assert.deepEqual(names(""), ["alfie", "bea", "cal", "maya", "everyone", "here"]);
  assert.deepEqual(names("", false), ["alfie", "bea", "cal", "maya", "mo", "sam_m"]);
  assert.deepEqual(names("every", false), []);
  assert.equal(names("").length, 6);
});

test("insertion replaces the token, adds a space and enforces the limit", () => {
  const token = mentionToken("hi @ma🙂", 6)!;
  assert.deepEqual(insertMention("hi @ma🙂", token, "maya"), { value: "hi @maya 🙂", caret: 9 });
  const long = `${"🙂".repeat(3994)} @m`;
  assert.equal(insertMention(long, mentionToken(long, long.length)!, "maya"), undefined);
  assert.ok(insertMention(long, mentionToken(long, long.length)!, "mo"));
});

test("only server-resolved names render as mentions", () => {
  const mentions = [{ type: "user", id: "u-maya", username: "maya" }, { type: "everyone" }, { type: "future" }];
  assert.deepEqual(mentionSegments("hey @Maya, @everyone @here @mo bob@maya.com (@maya)", mentions), [
    { text: "hey ", mention: false },
    { text: "@Maya", mention: true, user: { id: "u-maya", username: "maya" } },
    { text: ", ", mention: false },
    { text: "@everyone", mention: true },
    { text: " @here @mo bob@maya.com (", mention: false },
    { text: "@maya", mention: true, user: { id: "u-maya", username: "maya" } },
    { text: ")", mention: false },
  ]);
  assert.deepEqual(mentionSegments("@maya", undefined), [{ text: "@maya", mention: false }]);
  assert.deepEqual(mentionSegments("@maya", "bad" as never), [{ text: "@maya", mention: false }]);
  assert.deepEqual(mentionSegments(`@${"m".repeat(33)}`, [{ type: "user", id: "x", username: "m".repeat(33) }]), [{ text: `@${"m".repeat(33)}`, mention: false }]);
});

test("a message mentions the reader by id or by everyone/here from someone else", () => {
  const message = (author: string, mentions: unknown): ChatMessage => ({
    id: "m", channelId: "c", seq: "1", createdAt: "", clientMessageId: "x",
    author: { id: author, name: author, isGuest: false },
    content: { version: 1, type: "text", text: "", mentions: mentions as never },
  });
  assert.equal(mentionsAccount(message("u-mo", [{ type: "user", id: "u-maya", username: "maya" }]), "u-maya"), true);
  assert.equal(mentionsAccount(message("u-mo", [{ type: "user", id: "u-sam", username: "sam_m" }]), "u-maya"), false);
  assert.equal(mentionsAccount(message("u-mo", [{ type: "here" }]), "u-maya"), true);
  assert.equal(mentionsAccount(message("u-maya", [{ type: "everyone" }]), "u-maya"), false);
  assert.equal(mentionsAccount(message("u-mo", [{ type: "future", id: "u-maya" }]), "u-maya"), false);
  assert.equal(mentionsAccount(message("u-mo", undefined), "u-maya"), false);
  assert.equal(mentionsAccount(message("u-mo", [{ type: "everyone" }]), undefined), false);
});

test("profile cards prefer the most specific local match and flag yourself", () => {
  const directory: MentionCandidate[] = [
    { id: "u-maya", username: "maya", displayName: "Maya Lopez", avatarId: 31 },
    { id: "u-maya", username: "maya", displayName: "Stale Maya" },
  ];
  assert.deepEqual(mentionCardPerson({ id: "u-maya", username: "maya" }, directory, "u-me"),
    { id: "u-maya", username: "maya", displayName: "Maya Lopez", avatarId: 31, self: false });
  assert.deepEqual(mentionCardPerson({ id: "u-stranger", username: "sam" }, directory, "u-me"),
    { id: "u-stranger", username: "sam", displayName: undefined, avatarId: undefined, self: false });
  assert.equal(mentionCardPerson({ id: "u-me", username: "me" }, directory, "u-me").self, true);
  assert.equal(mentionCardPerson({ id: "u-me", username: "me" }, directory, undefined).self, false);
});
