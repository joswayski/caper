import { createFileRoute } from "@tanstack/react-router";
import { getInitialAccount } from "../account/server";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  loader: async () => ({
    account: await getInitialAccount(),
    initialNow: Date.now(),
    latestChanges: __LATEST_CHANGES__,
  }),
  component: HomeRoute,
  head: () => ({
    links: [
      {
        rel: "preload",
        as: "image",
        href: "/images/chat-preview-wide.webp?v=20260906",
        type: "image/webp",
        media: "(min-width: 1001px)",
      },
      {
        rel: "preload",
        as: "image",
        href: "/images/chat-preview-stacked.webp?v=20260906",
        type: "image/webp",
        media: "(max-width: 1000px)",
      },
    ],
  }),
});

function HomeRoute() {
  const { account, initialNow, latestChanges } = Route.useLoaderData();
  return <Home account={account} initialNow={initialNow} latestChanges={latestChanges} />;
}
