import { createFileRoute } from "@tanstack/react-router";
import { getInitialAccount } from "../account/server";
import { getPublicChannel } from "../spaces/server";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  loader: async () => {
    const [account, history] = await Promise.all([getInitialAccount(), getPublicChannel()]);
    return { account, history, initialNow: Date.now(), latestChanges: __LATEST_CHANGES__ };
  },
  component: HomeRoute,
});

function HomeRoute() {
  const { account, history, initialNow, latestChanges } = Route.useLoaderData();
  return <Home account={account} history={history} initialNow={initialNow} latestChanges={latestChanges} />;
}
