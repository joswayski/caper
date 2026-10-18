export interface Highlight {
  title: string;
  body: string;
}

export interface Release {
  sequence: number;
  revision: string;
  publishedAt: string;
  highlights: Highlight[];
}

export interface ReleaseHighlights {
  schema: 1;
  platform: "web";
  revision: string;
  sequence: number;
  releases: Release[];
}

export const sourceRevisionValid = (value: unknown): value is string =>
  typeof value === "string" && /^[0-9a-f]{40}$/u.test(value);
const sequenceValid = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) > 0;
const textValid = (value: unknown, maximum: number): value is string =>
  typeof value === "string" && !!value.trim() && [...value].length <= maximum && !/[<>\p{Cc}]/u.test(value);

/** Reject future/mismatched snapshots rather than announce unsupported features. */
export async function loadReleaseHighlights(revision: string): Promise<ReleaseHighlights> {
  if (!sourceRevisionValid(revision)) throw new Error("Release highlights are available in published builds.");
  const response = await fetch(`/api/whats-new?platform=web&revision=${revision}`, {
    credentials: "omit",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error("Release highlights aren’t available yet. Try again shortly.");
  const text = await response.text();
  if (new TextEncoder().encode(text).length > 65_536) throw new Error("Invalid release highlights.");
  let value: ReleaseHighlights;
  try {
    value = JSON.parse(text) as ReleaseHighlights;
  } catch {
    throw new Error("Invalid release highlights.");
  }
  let previous = 0;
  if (
    value?.schema !== 1 ||
    value.platform !== "web" ||
    value.revision !== revision ||
    !sequenceValid(value.sequence) ||
    !Array.isArray(value.releases) ||
    !value.releases.length ||
    value.releases.length > 30 ||
    value.releases.some((release) => {
      if (
        !release ||
        !sequenceValid(release.sequence) ||
        release.sequence <= previous ||
        release.sequence > value.sequence ||
        !sourceRevisionValid(release.revision) ||
        typeof release.publishedAt !== "string" ||
        !Number.isFinite(Date.parse(release.publishedAt)) ||
        !Array.isArray(release.highlights) ||
        release.highlights.length > 5 ||
        release.highlights.some(
          (highlight) => !highlight || !textValid(highlight.title, 100) || !textValid(highlight.body, 500),
        )
      )
        return true;
      previous = release.sequence;
      return false;
    }) ||
    value.releases.at(-1)?.sequence !== value.sequence ||
    value.releases.at(-1)?.revision !== revision
  )
    throw new Error("Invalid release highlights.");
  return value;
}

/** Undefined means a fresh installation: quietly establish its baseline. */
export function unreadReleaseHighlights(feed: ReleaseHighlights, acknowledged?: number): Release[] {
  return acknowledged === undefined
    ? []
    : feed.releases.filter((release) => release.sequence > acknowledged && release.highlights.length > 0).reverse();
}

/** Bundle the displayed releases by publication day, without changing read watermarks. */
export function releaseHighlightDays(releases: Release[]): { date: string; releases: Release[] }[] {
  const days = new Map<string, Release[]>();
  for (const release of releases) {
    if (!release.highlights.length) continue;
    const date = new Date(release.publishedAt).toISOString().slice(0, 10);
    const entries = days.get(date) ?? [];
    entries.push(release);
    days.set(date, entries);
  }
  return Array.from(days, ([date, releases]) => ({ date, releases }));
}
