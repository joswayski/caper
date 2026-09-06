import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig, searchForWorkspaceRoot } from "vite";

const repository = "joswayski/caper";

interface LatestChange {
  sha: string;
  title: string;
  url: string;
  committedAt: string;
}

interface GitHubCommit {
  sha: string;
  html_url: string;
  commit: {
    message: string;
    committer: { date: string };
  };
}

async function latestChanges(): Promise<LatestChange[]> {
  try {
    const response = await fetch(`https://api.github.com/repos/${repository}/commits?sha=main&per_page=10`, {
      headers: { Accept: "application/vnd.github+json" },
    });

    if (!response.ok) throw new Error(`GitHub returned ${response.status}`);

    const commits = await response.json() as GitHubCommit[];
    return commits.map((commit) => {
      const title = commit.commit.message.split("\n", 1)[0].replace(/\s*\(#\d+\)$/, "");
      const pullRequest = commit.commit.message.match(/\(#(\d+)\)\s*$/m)?.[1];

      return {
        sha: commit.sha,
        title,
        url: pullRequest ? `https://github.com/${repository}/pull/${pullRequest}` : commit.html_url,
        committedAt: commit.commit.committer.date,
      };
    });
  } catch (error) {
    console.warn("Unable to load latest Caper changes:", error);
    return [];
  }
}

export default defineConfig(async () => ({
  define: {
    __LATEST_CHANGES__: JSON.stringify(await latestChanges()),
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
}));
