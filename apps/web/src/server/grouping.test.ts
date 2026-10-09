import assert from "node:assert/strict";
import { test } from "vitest";
import { GROUP_WINDOW_MS, groupsWithPrevious } from "../chat/grouping.ts";

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
