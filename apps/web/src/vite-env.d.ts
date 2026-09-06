interface LatestChange {
  sha: string;
  title: string;
  url: string;
  committedAt: string;
}

declare const __LATEST_CHANGES__: LatestChange[];
