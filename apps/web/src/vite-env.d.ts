interface LatestChange {
  sha: string;
  title: string;
  url: string;
  committedAt: string;
  pullRequest: number | null;
}

declare const __LATEST_CHANGES__: LatestChange[];
