import assert from "node:assert/strict";
import { test } from "vitest";
import { channelNewMessagesId, GROUP_WINDOW_MS, groupsWithPrevious } from "../chat/grouping.ts";
import type { ChatMessage } from "../chat/types.ts";

const at = (minutes: number) => new Date(Date.UTC(2026, 9, 9, 12, 0) + minutes * 60_000).toISOString();
const by = (id: string, minutes: number, extra: { threadRootId?: string } = {}) => ({
  author: { id },
  createdAt: at(minutes),
  ...extra,
});

test("same person within five minutes groups", () => {
  assert.equal(GROUP_WINDOW_MS, 300_000);
  assert.equal(groupsWithPrevious(by("a", 0), by("a", 5)), true);
  assert.equal(groupsWithPrevious(by("a", 0), by("a", 5.01)), false);
  assert.equal(groupsWithPrevious(by("a", 0), by("b", 1)), false);
  assert.equal(groupsWithPrevious(undefined, by("a", 0)), false);
});

test("broadcast thread replies keep their header in the channel, not in the thread", () => {
  assert.equal(groupsWithPrevious(by("a", 0), by("a", 1, { threadRootId: "r" })), false);
  assert.equal(groupsWithPrevious(by("a", 0), by("a", 1, { threadRootId: "r" }), { inThread: true }), true);
});

test("a new day starts a new group", () => {
  const late = { author: { id: "a" }, createdAt: new Date(2026, 9, 9, 23, 58).toISOString() };
  const early = { author: { id: "a" }, createdAt: new Date(2026, 9, 10, 0, 1).toISOString() };
  assert.equal(groupsWithPrevious(late, early), false);
});

test("channel unread divider uses the entry boundary, not live arrivals or a truncated history edge", () => {
  const row = (seq: string, author = "other", extra: Partial<ChatMessage> = {}): ChatMessage => ({
    id: seq,
    seq,
    channelId: "channel",
    clientMessageId: seq,
    author: { id: author, name: author, isGuest: false },
    content: { version: 1, type: "text", text: seq },
    createdAt: at(0),
    ...extra,
  });
  const rows = [
    row("10"),
    row("11", "me"),
    row("12", "blocked"),
    row("13", "other", { threadRootId: "root" }),
    row("15"),
    row("16"),
  ];
  const entry = { readSeq: "10", lastSeq: "15" };
  const divider = (messages: ChatMessage[], boundary = entry, more = true) =>
    channelNewMessagesId(messages, boundary, more, "me", new Set(["blocked"]));
  assert.equal(divider(rows), "15");
  assert.equal(divider(rows.slice(1)), "15");
  assert.equal(divider(rows.slice(2)), undefined);
  assert.equal(divider(rows.slice(2), entry, false), "15");
  assert.equal(divider(rows.slice(-1), entry, false), undefined);
  assert.equal(channelNewMessagesId(rows, undefined, false, "me", new Set()), undefined);
  assert.equal(channelNewMessagesId(rows, { lastSeq: "15" }, false, "me", new Set()), undefined);
  assert.equal(
    divider([row("9007199254740993")], { readSeq: "9007199254740992", lastSeq: "9007199254740993" }),
    "9007199254740993",
  );
});
