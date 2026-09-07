import { createFileRoute, redirect } from "@tanstack/react-router";
import { useAuth } from "@workos/authkit-tanstack-react-start/client";
import Call from "../pages/Call";
import { loadAccount } from "../server/account";
import "../pages/account.css";

export const Route = createFileRoute("/live")({
  loader: async () => {
    const account = await loadAccount();
    if (!account.username) throw redirect({ href: "/profile" });
    return account;
  },
  component: Live,
  head: () => ({ meta: [{ title: "General · Caper" }] }),
});

function Live() {
  const account = Route.useLoaderData();
  const { signOut } = useAuth();
  return <>
    <nav className="account-nav" aria-label="Account">
      <a href="/profile">@{account.username}</a>
      <button className="account-secondary" onClick={() => signOut({ returnTo: "/login" })}>Sign out</button>
    </nav>
    <Call />
  </>;
}
