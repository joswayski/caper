import assert from "node:assert/strict";
import { test } from "vitest";
import {
  draftMentionSegments,
  excludePeople,
  insertMention,
  mentionCardPerson,
  mentionName,
  mentionSegments,
  mentionSuggestions,
  mentionToken,
  mentionsAccount,
  reportsFeedback,
  rankPeople,
  withInvited,
  type MentionCandidate,
} from "../chat/mentions.ts";
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
  assert.deepEqual(names(""), ["feedback", "alfie", "bea", "cal", "everyone", "here"]);
  assert.deepEqual(names("", false), ["feedback", "alfie", "bea", "cal", "maya", "mo"]);
  assert.deepEqual(names("every", false), []);
  assert.equal(names("").length, 6);
});

test("username fields rank people like @ suggestions, without members, invitations or specials", () => {
  const names = (people: MentionCandidate[], query: string) =>
    rankPeople(people, query).map((person) => person.username);
  assert.deepEqual(names(members, "MAYA"), ["maya", "sam_m", "zz_maya"]);
  assert.deepEqual(names(members, "e"), ["alfie", "bea"]);
  assert.deepEqual(names(members, "every"), []);
  assert.equal(names(members, "").length, 6);
  const candidates = excludePeople(members, [{ id: "u-maya" }], [{ id: "u-zz" }, { id: "missing" }]);
  assert.deepEqual(names(candidates, "maya"), ["sam_m"]);
  assert.deepEqual(excludePeople(members), members);
});

test("pending invitees stay in username-field matches, marked as already invited", () => {
  const invited = { id: "u-zz", username: "zz_maya", displayName: "Zed", avatarId: 4, owner: false };
  const candidates = withInvited(excludePeople(members, [{ id: "u-maya" }], [invited]), [invited]);
  assert.deepEqual(
    rankPeople(candidates, "maya").map((person) => [person.username, !!person.invited]),
    [
      ["sam_m", false],
      ["zz_maya", true],
    ],
  );
  assert.deepEqual(candidates.at(-1), {
    id: "u-zz",
    username: "zz_maya",
    displayName: "Zed",
    avatarId: 4,
    invited: true,
  });
  assert.deepEqual(withInvited(members, []), members);
});

test("insertion replaces the token, adds a space and enforces the limit", () => {
  const token = mentionToken("hi @ma🙂", 6)!;
  assert.deepEqual(insertMention("hi @ma🙂", token, "maya"), { value: "hi @maya 🙂", caret: 9 });
  const long = `${"🙂".repeat(3994)} @m`;
  assert.equal(insertMention(long, mentionToken(long, long.length)!, "maya"), undefined);
  assert.ok(insertMention(long, mentionToken(long, long.length)!, "mo"));
});

test("drafts highlight complete known names and team aliases, with channel specials only in channels", () => {
  const text = "🙂 @Maya, @ma @missing bob@maya.com @@maya (@feedback)\n@Here @everyone @feedbacks";
  const channel = draftMentionSegments(text, members, true);
  assert.equal(channel.map((segment) => segment.text).join(""), text);
  assert.deepEqual(
    channel.filter((segment) => segment.mention).map((segment) => segment.text),
    ["@Maya", "@feedback", "@Here", "@everyone"],
  );
  assert.deepEqual(
    draftMentionSegments(text, members, false)
      .filter((segment) => segment.mention)
      .map((segment) => segment.text),
    ["@Maya", "@feedback"],
  );
  assert.deepEqual(
    draftMentionSegments("@maya @Admin")
      .filter((segment) => segment.mention)
      .map((segment) => segment.text),
    ["@Admin"],
  );
  assert.deepEqual(
    draftMentionSegments(`@${"a".repeat(33)}`, [{ id: "long", username: "a".repeat(33), displayName: "Long" }]).filter(
      (segment) => segment.mention,
    ),
    [],
  );
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
  assert.deepEqual(mentionSegments(`@${"m".repeat(33)}`, [{ type: "user", id: "x", username: "m".repeat(33) }]), [
    { text: `@${"m".repeat(33)}`, mention: false },
  ]);
});

test("a message mentions the reader by id or by everyone/here from someone else", () => {
  const message = (author: string, mentions: unknown): ChatMessage => ({
    id: "m",
    channelId: "c",
    seq: "1",
    createdAt: "",
    clientMessageId: "x",
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
  assert.deepEqual(mentionCardPerson({ id: "u-maya", username: "maya" }, directory, "u-me"), {
    id: "u-maya",
    username: "maya",
    displayName: "Maya Lopez",
    avatarId: 31,
    self: false,
  });
  assert.deepEqual(mentionCardPerson({ id: "u-stranger", username: "sam" }, directory, "u-me"), {
    id: "u-stranger",
    username: "sam",
    displayName: undefined,
    avatarId: undefined,
    self: false,
  });
  assert.equal(mentionCardPerson({ id: "u-me", username: "me" }, directory, "u-me").self, true);
  assert.equal(mentionCardPerson({ id: "u-me", username: "me" }, directory, undefined).self, false);
  assert.equal(mentionCardPerson({ id: "u-maya", admin: true }, directory, "u-me").admin, true);
  assert.equal(
    mentionCardPerson({ id: "u-maya" }, [...directory, { ...directory[0], admin: true }], "u-me").admin,
    true,
  );
  assert.equal(mentionCardPerson({ id: "u-maya" }, [...directory, { ...directory[0], team: true }], "u-me").team, true);
  assert.equal(mentionCardPerson({ id: "u-stranger" }, [{ ...directory[0], admin: true }], "u-me").admin, undefined);
});

test("pinner profiles match IDs, use canonical identities, and preserve unknown author metadata", () => {
  const author = { id: "pinner", name: "Same Name", avatarId: 7 };
  const directory = [
    { id: "message-author", username: "wrong_person", displayName: "Same Name", avatarId: 2 },
    { id: "pinner", username: "renamed_pinner", displayName: "Current Name", avatarId: null },
  ];
  assert.deepEqual(mentionCardPerson(author, directory, "message-author"), {
    id: "pinner",
    username: "renamed_pinner",
    displayName: "Current Name",
    avatarId: null,
    self: false,
  });
  assert.deepEqual(mentionCardPerson(author, directory.slice(0, 1), "message-author"), {
    id: "pinner",
    username: undefined,
    displayName: "Same Name",
    avatarId: 7,
    self: false,
  });
  assert.equal(mentionCardPerson(author, [], "pinner").self, true);
});

test("team names are suggested everywhere once typed, completing the name, and render as pills", () => {
  const names = (query: string, specials = true) => mentionSuggestions(members, query, specials).map(mentionName);
  assert.deepEqual(names("feed", false), ["feedback"]);
  assert.deepEqual(names("cap"), ["caper"]);
  assert.deepEqual(names("adm"), ["admin"]);
  assert.deepEqual(names("admins"), ["admins"]);
  assert.deepEqual(names("c", false), ["cal", "caper"]);
  assert.deepEqual(names("mo"), ["mo", "mod"]);
  assert.equal(names("")[0], "feedback", "a bare @ leads with the team, once");
  assert.equal(names("").filter((name) => name === "feedback").length, 1);
  assert.deepEqual(names("a", false).at(-1), "admin", "typed queries keep people first");
  assert.deepEqual(
    mentionSegments("ping @Admin and @mods", [{ type: "feedback" }])
      .filter((s) => s.mention)
      .map((s) => s.text),
    ["@Admin", "@mods"],
  );
  const text = "@Feedback uploads fail, @caper @feedbacks";
  const segments = mentionSegments(text, [{ type: "feedback" }]);
  assert.deepEqual(
    segments.filter((segment) => segment.mention).map((segment) => [segment.text, segment.user]),
    [
      ["@Feedback", undefined],
      ["@caper", undefined],
    ],
  );
  assert.deepEqual(mentionSegments(text, []), [{ text, mention: false }], "unreported text stays plain");
  const message = { content: { mentions: [{ type: "feedback" }] } } as unknown as ChatMessage;
  assert.ok(reportsFeedback(message));
  assert.ok(!reportsFeedback({ content: {} } as unknown as ChatMessage));
});
