import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig, searchForWorkspaceRoot } from "vite";

const repository = "joswayski/caper";
const changeCount = 10;
/** Fetch extra commits so Dependabot merges can be dropped without under-filling the list. */
const fetchCount = 30;

interface LatestChange {
  sha: string;
  title: string;
  url: string;
  committedAt: string;
  pullRequest: number | null;
}

interface GitHubCommit {
  sha: string;
  html_url: string;
  author: { login: string } | null;
  commit: {
    message: string;
    committer: { date: string } | null;
    author: { name?: string; date: string } | null;
  };
}

function githubRequestHeaders(): Record<string, string> {
  const token = process.env.GITHUB_TOKEN?.trim();
  return {
    Accept: "application/vnd.github+json",
    "User-Agent": "caper-web-build",
    "X-GitHub-Api-Version": "2022-11-28",
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
  };
}

function pullRequestNumber(title: string) {
  return title.match(/\(#(\d+)\)$/u)?.[1] ?? title.match(/^Merge pull request #(\d+)/u)?.[1] ?? null;
}

function isDependabotCommit(entry: GitHubCommit): boolean {
  const login = entry.author?.login?.toLowerCase() ?? "";
  if (login === "dependabot[bot]" || login.startsWith("dependabot")) return true;

  const authorName = entry.commit.author?.name?.toLowerCase() ?? "";
  return authorName === "dependabot[bot]" || authorName.startsWith("dependabot");
}

function toLatestChange(entry: GitHubCommit): LatestChange {
  const title = entry.commit.message.split("\n", 1)[0]?.trim();
  const committedAt = entry.commit.committer?.date ?? entry.commit.author?.date;
  if (!entry.sha || !entry.html_url || !title || !committedAt) {
    throw new Error("GitHub returned an incomplete commit entry");
  }

  const pullRequest = pullRequestNumber(title);
  return {
    sha: entry.sha,
    title: pullRequest ? title.replace(/\s+\(#\d+\)$/u, "") : title,
    url: pullRequest ? `https://github.com/${repository}/pull/${pullRequest}` : entry.html_url,
    committedAt,
    pullRequest: pullRequest ? Number(pullRequest) : null,
  };
}

async function latestChanges(): Promise<LatestChange[]> {
  const url = new URL(`https://api.github.com/repos/${repository}/commits`);
  url.searchParams.set("sha", "main");
  url.searchParams.set("per_page", String(fetchCount));

  const response = await fetch(url, {
    headers: githubRequestHeaders(),
    signal: AbortSignal.timeout(30_000),
  });
  if (!response.ok) throw new Error(`GitHub history request failed with ${response.status}`);

  const entries = await response.json() as GitHubCommit[];
  if (!Array.isArray(entries) || entries.length === 0) {
    throw new Error("GitHub returned no commits for main");
  }

  const productChanges = entries
    .filter((entry) => !isDependabotCommit(entry))
    .map(toLatestChange)
    .slice(0, changeCount);
  if (productChanges.length === 0) {
    throw new Error("GitHub returned no non-Dependabot commits for main");
  }

  return productChanges;
}

export default defineConfig(async () => {
  const changes = await latestChanges();
  console.log(`Fetched ${changes.length} latest changes from the GitHub API.`);

  return {
  define: {
    __LATEST_CHANGES__: JSON.stringify(changes),
  },
  plugins: [
    tanstackStart({
      prerender: { enabled: false },
      server: { build: { inlineCss: true } },
    }),
    react(),
    nitro({
      routeRules: {
        "/audio/deepfilter-v1/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/audio/noise-v1/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/audio/rnnoise-v1/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/audio/dpdfnet2-v1/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/audio/dpdfnet8-v1/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/assets/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/images/**": {
          headers: { "cache-control": "public, max-age=86400" },
        },
        "/brand/**": {
          headers: { "cache-control": "public, max-age=86400" },
        },
      },
    }),
  ],
  server: {
    port: Number(process.env.PORT ?? 5174),
    allowedHosts: [".onamp.dev"],
    fs: { allow: [searchForWorkspaceRoot(import.meta.dirname)] },
  },
  };
});
