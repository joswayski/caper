import { createFileRoute } from "@tanstack/react-router";
import { getInitialAccount } from "../account/server";
import { getPublicDemo } from "../spaces/server";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  loader: async () => {
    const [account, demo] = await Promise.all([getInitialAccount(), getPublicDemo()]);
    return { account, demo, initialNow: Date.now(), latestChanges: __LATEST_CHANGES__ };
  },
  component: HomeRoute,
});

function HomeRoute() {
  const { account, demo, initialNow, latestChanges } = Route.useLoaderData();
  return <Home account={account} demo={demo} initialNow={initialNow} latestChanges={latestChanges} />;
}
