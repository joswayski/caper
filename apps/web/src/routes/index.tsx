import { createFileRoute } from "@tanstack/react-router";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  loader: () => ({
    initialNow: Date.now(),
    latestChanges: __LATEST_CHANGES__,
  }),
  staleTime: Number.POSITIVE_INFINITY,
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
  const { initialNow, latestChanges } = Route.useLoaderData();
  return <Home initialNow={initialNow} latestChanges={latestChanges} />;
}
