import { test, expect, vi } from "vitest";
import {
  loadReleaseHighlights,
  releaseHighlightDays,
  unreadReleaseHighlights,
  type ReleaseHighlights,
} from "./whats-new";

const revision = "a".repeat(40);
const feed: ReleaseHighlights = {
  schema: 1,
  platform: "web",
  revision,
  sequence: 4,
  releases: [
    {
      sequence: 1,
      revision: "b".repeat(40),
      publishedAt: "2026-10-08T12:00:00Z",
      highlights: [{ title: "Earlier update", body: "Already acknowledged." }],
    },
    {
      sequence: 2,
      revision: "c".repeat(40),
      publishedAt: "2026-10-09T12:00:00Z",
      highlights: [{ title: "Edit messages", body: "View earlier versions." }],
    },
    { sequence: 3, revision: "d".repeat(40), publishedAt: "2026-10-10T12:00:00Z", highlights: [] },
    {
      sequence: 4,
      revision,
      publishedAt: "2026-10-11T12:00:00Z",
      highlights: [{ title: "Channel pins", body: "Find important messages together." }],
    },
  ],
};

test("release highlights quietly baseline new installs and catch up without repeating acknowledged or silent releases", async () => {
  vi.stubGlobal("fetch", async () => Response.json(feed));
  const loaded = await loadReleaseHighlights(revision);
  expect(unreadReleaseHighlights(loaded)).toEqual([]);
  expect(unreadReleaseHighlights(loaded, 1).map((release) => release.sequence)).toEqual([4, 2]);
  expect(unreadReleaseHighlights(loaded, 2).map((release) => release.sequence)).toEqual([4]);
  expect(unreadReleaseHighlights(loaded, 4)).toEqual([]);
  expect(unreadReleaseHighlights(loaded, 7)).toEqual([]); // Rolled-back builds stay quiet.
  expect(loaded.releases[0].sequence).toBe(1); // Presentation never mutates chronological history.
});

test("release history bundles a publication day while preserving highlights, order, and unread boundaries", () => {
  const history = structuredClone(feed);
  history.releases[1].publishedAt = "2026-10-11T00:01:00Z";
  history.releases[2].publishedAt = "2026-10-11T08:00:00Z";
  history.releases[3].highlights.push({ title: "More pin details", body: "See who pinned a message." });
  const before = structuredClone(history);
  expect(releaseHighlightDays([...history.releases].reverse())).toEqual([
    {
      date: "2026-10-11",
      releases: [history.releases[3], history.releases[1]],
    },
    { date: "2026-10-08", releases: [history.releases[0]] },
  ]);
  expect(releaseHighlightDays(unreadReleaseHighlights(history, 2))).toEqual([
    {
      date: "2026-10-11",
      releases: [history.releases[3]],
    },
  ]);
  expect(releaseHighlightDays([])).toEqual([]);
  expect(history).toEqual(before);
});

test("release highlights fail closed on wrong platform/revision, inconsistent history, unsafe text, and unavailable publication", async () => {
  const invalid = [
    { ...feed, platform: "android" },
    { ...feed, revision: "f".repeat(40) },
    { ...feed, sequence: 5 },
    { ...feed, releases: [feed.releases[1], feed.releases[0], ...feed.releases.slice(2)] },
    { ...feed, releases: [] },
    { ...feed, releases: [{ ...feed.releases[3], highlights: [{ title: "<script>", body: "Invalid." }] }] },
    { ...feed, releases: [{ ...feed.releases[3], highlights: [{ title: "a".repeat(101), body: "Invalid." }] }] },
  ];
  for (const value of invalid) {
    vi.stubGlobal("fetch", async () => Response.json(value));
    await expect(loadReleaseHighlights(revision)).rejects.toThrow("Invalid release highlights.");
  }
  vi.stubGlobal("fetch", async () => new Response(null, { status: 502 }));
  await expect(loadReleaseHighlights(revision)).rejects.toThrow("aren’t available yet");
  await expect(loadReleaseHighlights("../latest")).rejects.toThrow("published builds");
});
