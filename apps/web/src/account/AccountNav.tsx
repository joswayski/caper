import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { getAccount, logout, type Account } from "./client";

export default function AccountNav({
  account: initialAccount,
  profileLabel,
}: {
  account?: Account | null;
  profileLabel?: string;
}) {
  const [account, setAccount] = useState(initialAccount);
  const [loggingOut, setLoggingOut] = useState(false);
  const [logoutFailed, setLogoutFailed] = useState(false);

  useEffect(() => {
    if (initialAccount !== undefined) return;

    let current = true;
    void getAccount()
      .then((result) => {
        if (current) setAccount(result);
      })
      .catch(() => {
        if (current) setAccount(null);
      });
    return () => {
      current = false;
    };
  }, [initialAccount]);

  if (account === undefined)
    return <span className="size-[7px] rounded-full bg-border" role="img" aria-label="Checking account" />;
  if (account === null)
    return (
      <a
        className="rounded-[4px] py-2.5 text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        href="/login"
      >
        Sign in
      </a>
    );

  return (
    <div className="flex min-w-0 items-center gap-4 max-[560px]:gap-2.5">
      <Link
        className="shrink-0 rounded-[4px] py-2.5 text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        to="/spaces"
        preload={account.username && account.displayName ? "render" : false}
      >
        Spaces
      </Link>
      <Link
        className="min-w-0 truncate rounded-[4px] py-2.5 text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        to="/profile"
      >
        {profileLabel ?? (account.username ? `@${account.username}` : "Finish setup")}
      </Link>
      <button
        className="shrink-0 cursor-pointer rounded-[4px] border-0 bg-transparent px-0 py-2.5 text-sm font-bold text-content-muted transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        type="button"
        disabled={loggingOut}
        onClick={() => {
          setLoggingOut(true);
          setLogoutFailed(false);
          logout().then(
            () => window.location.assign("/"),
            () => {
              // Stay signed in and say so, instead of a click that silently did nothing.
              setLoggingOut(false);
              setLogoutFailed(true);
            },
          );
        }}
      >
        {loggingOut ? (
          "Logging out…"
        ) : logoutFailed ? (
          // Narrow screens show only the action, so the profile link keeps its room.
          <span role="alert">
            <span className="max-[560px]:sr-only">Couldn’t log out. </span>Try again
          </span>
        ) : (
          "Log out"
        )}
      </button>
    </div>
  );
}
