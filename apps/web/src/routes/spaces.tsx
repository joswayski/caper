import { createFileRoute, redirect } from "@tanstack/react-router";
import { getAccount } from "../account/client";
import Spaces from "../spaces/Spaces";
import { listSpaces } from "../spaces/client";

export const Route = createFileRoute("/spaces")({
  ssr: false,
  loader: async () => {
    const account = await getAccount();
    if (!account) throw redirect({ to: "/login", replace: true });
    if (!account.username || !account.displayName) throw redirect({ to: "/profile", replace: true });
    // Keep the current page visible until the destination knows whether to show
    // first-space setup or the conversation UI, rather than flashing its shell.
    const initialSpaceList = await listSpaces().catch(() => undefined);
    return { initialAccount: account, initialSpaceList };
  },
  component: SpacesPage,
});

function SpacesPage() {
  return <Spaces {...Route.useLoaderData()} />;
}
