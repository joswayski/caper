import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeader, setResponseHeader } from "@tanstack/react-start/server";
import { getInitialAccount } from "../account/server";
import { detectDownloadPlatform } from "../downloads";
import Home from "../pages/Home";
import { fetchHighlights, type LatestChange } from "../release-highlights";

// Highlights publish after each web rollout, so read them live instead of
// waiting for the next build. Requests share one cached copy per pod.
const HIGHLIGHTS_TTL_MS = 60_000;
const HIGHLIGHTS_TIMEOUT_MS = 3_000;
let highlights: { at: number; changes: LatestChange[] } | undefined;
let refreshing: Promise<void> | undefined;

function refreshHighlights() {
  refreshing ??= fetchHighlights(AbortSignal.timeout(HIGHLIGHTS_TIMEOUT_MS))
    .then((changes) => {
      highlights = { at: Date.now(), changes };
    })
    .catch(() => {
      // Keep the last good copy, and wait a full TTL before asking GitHub again.
      highlights = { at: Date.now(), changes: highlights?.changes ?? [] };
    })
    .finally(() => {
      refreshing = undefined;
    });
  return refreshing;
}

const getLatestChanges = createServerFn({ method: "GET" }).handler(async (): Promise<LatestChange[]> => {
  if (!highlights) await refreshHighlights();
  else if (Date.now() - highlights.at > HIGHLIGHTS_TTL_MS) void refreshHighlights();
  // Built-in list (highlights or commit titles at build time) covers a cold or failed fetch.
  return highlights?.changes.length ? highlights.changes : __LATEST_CHANGES__;
});

const getDownloadPlatform = createServerFn({ method: "GET" }).handler(() => {
  setResponseHeader("Vary", "User-Agent, Sec-CH-UA-Platform, Sec-CH-UA-Mobile");
  setResponseHeader("Accept-CH", "Sec-CH-UA-Platform, Sec-CH-UA-Mobile");
  setResponseHeader("Cache-Control", "private, no-store");
  return detectDownloadPlatform({
    userAgent: getRequestHeader("user-agent") ?? "",
    platform: getRequestHeader("sec-ch-ua-platform") ?? "",
    mobile: getRequestHeader("sec-ch-ua-mobile") === "?1",
  });
});

export const Route = createFileRoute("/")({
  loader: async () => {
    const [account, downloadPlatform, latestChanges] = await Promise.all([
      getInitialAccount(),
      getDownloadPlatform(),
      getLatestChanges(),
    ]);
    return { account, downloadPlatform, initialNow: Date.now(), latestChanges };
  },
  component: HomeRoute,
});

function HomeRoute() {
  return <Home {...Route.useLoaderData()} />;
}
