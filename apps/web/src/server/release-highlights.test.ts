import assert from "node:assert/strict";
import { test } from "vitest";
import { parseHighlights } from "../release-highlights";

const sha = (digit: string) => digit.repeat(40);

test("release highlights list newest releases first and skip malformed entries", () => {
  const changes = parseHighlights({
    releases: [
      { revision: sha("a"), publishedAt: "2026-10-18T08:00:00Z", highlights: [{ title: "Older", body: "First." }] },
      { revision: sha("b"), publishedAt: "2026-10-19T08:00:00Z", highlights: [] },
      { revision: "not-a-sha", publishedAt: "2026-10-19T09:00:00Z", highlights: [{ title: "Bad", body: "Skipped." }] },
      {
        revision: sha("c"),
        publishedAt: "2026-10-19T10:00:00Z",
        highlights: [
          { title: "Newer", body: "Second." },
          { title: "", body: "Empty title." },
          { title: "Long", body: "x".repeat(501) },
        ],
      },
    ],
  });
  assert.deepEqual(
    changes.map(({ title, sha: revision, committedAt }) => [title, revision, committedAt]),
    [
      ["Newer", sha("c"), "2026-10-19T10:00:00Z"],
      ["Older", sha("a"), "2026-10-18T08:00:00Z"],
    ],
  );
  assert.equal(changes[0]?.url, `https://github.com/joswayski/caper/commit/${sha("c")}`);
});

test("release highlights keep at most eight and reject a feed without releases", () => {
  const releases = Array.from({ length: 6 }, (_, index) => ({
    revision: sha(String(index)),
    publishedAt: `2026-10-1${index}T00:00:00Z`,
    highlights: [
      { title: `A${index}`, body: "One." },
      { title: `B${index}`, body: "Two." },
    ],
  }));
  assert.equal(parseHighlights({ releases }).length, 8);
  assert.throws(() => parseHighlights({}));
  assert.throws(() => parseHighlights(null));
});
