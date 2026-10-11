import { createFileRoute, redirect } from "@tanstack/react-router";
import { getAccount } from "../account/client";
import Spaces from "../spaces/Spaces";
import { listSpaces } from "../spaces/client";
import SpacesLoading from "../spaces/SpacesLoading";

export const Route = createFileRoute("/spaces")({
  ssr: false,
  pendingComponent: SpacesLoading,
  pendingMs: 150,
  pendingMinMs: 0,
  loader: async () => {
    const account = await getAccount();
    if (!account) throw redirect({ to: "/login", replace: true });
    if (!account.username || !account.displayName) throw redirect({ to: "/profile", replace: true });
    // The server-rendered skeleton stays visible through account/list loading
    // and the first conversation, including direct visits and page reloads.
    const initialSpaceList = await listSpaces().catch(() => undefined);
    return { initialAccount: account, initialSpaceList };
  },
  component: SpacesPage,
});

function SpacesPage() {
  return <Spaces {...Route.useLoaderData()} />;
}
