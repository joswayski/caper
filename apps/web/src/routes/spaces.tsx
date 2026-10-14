import { createFileRoute, redirect, useLocation } from "@tanstack/react-router";
import { getAccount } from "../account/client";
import Spaces from "../spaces/Spaces";
import { listSpaces } from "../spaces/client";
import { prepareSpaceLanding } from "../spaces/navigation";
import SpacesLoading from "../spaces/SpacesLoading";

export const Route = createFileRoute("/spaces")({
  ssr: false,
  pendingComponent: SpacesLoading,
  pendingMs: 150,
  pendingMinMs: 0,
  preloadStaleTime: 5_000,
  preloadGcTime: 5_000,
  gcTime: 0,
  // Spaces owns in-room URL changes and live navigation. They must not rerun
  // the entry loader or replace a mounted conversation with a pending shell.
  shouldReload: ({ cause }) => (cause === "stay" ? false : undefined),
  loader: {
    staleReloadMode: "blocking",
    handler: async ({ location }) => {
      const account = await getAccount();
      if (!account) throw redirect({ to: "/login", replace: true });
      if (!account.username || !account.displayName) throw redirect({ to: "/profile", replace: true });
      const initialSpaceList = await listSpaces().catch(() => undefined);
      const landing = initialSpaceList
        ? await prepareSpaceLanding(account.id, initialSpaceList.spaces, new URLSearchParams(location.searchStr))
        : undefined;
      return { initialAccount: account, initialSpaceList, ...landing, entrySearch: location.searchStr };
    },
  },
  component: SpacesPage,
});

function SpacesPage() {
  const { entrySearch, ...initial } = Route.useLoaderData();
  const search = useLocation({ select: (location) => location.searchStr });
  // A default-entry preload cannot override an explicit link to another room.
  return (
    <Spaces
      {...(entrySearch === search
        ? initial
        : { initialAccount: initial.initialAccount, initialSpaceList: initial.initialSpaceList })}
    />
  );
}
