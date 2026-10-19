interface LatestChange {
  sha: string;
  title: string;
  /** Set for Claude-written release highlights; commit-title fallbacks have none. */
  body: string | null;
  url: string;
  committedAt: string;
  pullRequest: number | null;
}

declare const __LATEST_CHANGES__: LatestChange[];
declare const __SOURCE_REVISION__: string;
