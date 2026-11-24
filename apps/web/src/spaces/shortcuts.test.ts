import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { adjacentConversation, rankSwitcher, switcherScore } from "./shortcuts.ts";

// Every client runs these same cases; see shared/shortcuts/switcher-cases.json.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/shortcuts/switcher-cases.json", import.meta.url), "utf8"),
) as {
  match: { query: string; text: string; score: number | null }[];
  rank: { query: string; items: { name: string; alternates: string[]; unread: boolean }[]; order: number[] }[];
  adjacent: {
    unread: boolean[];
    current: number | null;
    forward: boolean;
    unreadOnly: boolean;
    result: number | null;
  }[];
};

test("quick switcher matching follows the shared cases", () => {
  for (const { query, text, score } of shared.match)
    assert.equal(switcherScore(query, text), score, `${query} ~ ${text}`);
  for (const { query, items, order } of shared.rank)
    assert.deepEqual(
      rankSwitcher(items, query).map((item) => items.indexOf(item)),
      order,
      query,
    );
});

test("conversation stepping follows the shared cases", () => {
  for (const { unread, current, forward, unreadOnly, result } of shared.adjacent)
    assert.equal(
      adjacentConversation(unread, current, forward, unreadOnly),
      result,
      JSON.stringify({ unread, current, forward, unreadOnly }),
    );
});
