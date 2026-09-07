import { createFileRoute, redirect } from "@tanstack/react-router";
import Home from "../pages/Home";
import { loadAccount } from "../server/account";

export const Route = createFileRoute("/")({
  loader: async () => {
    const account = await loadAccount();
    if (!account.username) throw redirect({ href: "/profile" });
    return { initialNow: Date.now(), latestChanges: __LATEST_CHANGES__ };
  },
  component: HomeRoute,
  head: () => ({
    links: [
      { rel: "preload", as: "image", href: "/images/chat-preview-wide.webp", type: "image/webp", media: "(min-width: 1001px)" },
      { rel: "preload", as: "image", href: "/images/chat-preview-stacked.webp", type: "image/webp", media: "(max-width: 1000px)" },
    ],
  }),
});

function HomeRoute() {
  const { initialNow, latestChanges } = Route.useLoaderData();
  return <Home initialNow={initialNow} latestChanges={latestChanges} />;
}
