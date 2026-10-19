const repository = "joswayski/caper";
/** Claude-written highlights shown in place of commit titles once any are published. */
const highlightCount = 8;
export const highlightsFeed = `https://github.com/${repository}/releases/download/whats-new-web/latest.json`;

export interface LatestChange {
  sha: string;
  title: string;
  /** Set for Claude-written release highlights; commit-title fallbacks have none. */
  body: string | null;
  url: string;
  committedAt: string;
  pullRequest: number | null;
}

function plainText(value: unknown, max: number): value is string {
  return typeof value === "string" && value.trim().length > 0 && [...value].length <= max;
}

/** Newest published web highlights, newest release first; malformed entries are skipped. */
export function parseHighlights(feed: unknown): LatestChange[] {
  const releases = (feed as { releases?: unknown } | null)?.releases;
  if (!Array.isArray(releases)) throw new Error("Release highlights feed has no releases");

  const highlights: LatestChange[] = [];
  for (const release of [...releases].reverse()) {
    const { revision, publishedAt, highlights: entries } = release ?? {};
    if (typeof revision !== "string" || !/^[0-9a-f]{40}$/u.test(revision)) continue;
    if (typeof publishedAt !== "string" || Number.isNaN(Date.parse(publishedAt)) || !Array.isArray(entries)) continue;
    for (const entry of entries) {
      if (!plainText(entry?.title, 100) || !plainText(entry?.body, 500)) continue;
      highlights.push({
        sha: revision,
        title: entry.title,
        body: entry.body,
        url: `https://github.com/${repository}/commit/${revision}`,
        committedAt: publishedAt,
        pullRequest: null,
      });
    }
    if (highlights.length >= highlightCount) break;
  }
  return highlights.slice(0, highlightCount);
}

export async function fetchHighlights(signal: AbortSignal): Promise<LatestChange[]> {
  const response = await fetch(highlightsFeed, { headers: { "User-Agent": "caper-web" }, signal });
  if (!response.ok) throw new Error(`Release highlights request failed with ${response.status}`);
  return parseHighlights(await response.json());
}
