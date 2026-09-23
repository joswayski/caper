import { createFileRoute } from "@tanstack/react-router";
import { getInitialAccount } from "../account/server";
import { getPublicDemoHref } from "../spaces/server";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  loader: async () => {
    const [account, demoHref] = await Promise.all([getInitialAccount(), getPublicDemoHref()]);
    return { account, demoHref, initialNow: Date.now(), latestChanges: __LATEST_CHANGES__ };
  },
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
  const { account, demoHref, initialNow, latestChanges } = Route.useLoaderData();
  return <Home account={account} demoHref={demoHref} initialNow={initialNow} latestChanges={latestChanges} />;
}
