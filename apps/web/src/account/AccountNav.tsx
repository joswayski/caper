import { useEffect, useState } from "react";
import { getAccount, logout, type Account } from "./client";

export default function AccountNav({
  account: initialAccount,
  profileLabel,
}: {
  account?: Account | null;
  profileLabel?: string;
}) {
  const [account, setAccount] = useState(initialAccount);

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
      <a
        className="shrink-0 rounded-[4px] py-2.5 text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        href="/spaces"
      >
        Spaces
      </a>
      <a
        className="min-w-0 truncate rounded-[4px] py-2.5 text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        href="/profile"
      >
        {profileLabel ?? (account.username ? `@${account.username}` : "Finish account")}
      </a>
      <button
        className="shrink-0 cursor-pointer rounded-[4px] border-0 bg-transparent px-0 py-2.5 text-sm font-bold text-content-muted transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 max-[560px]:text-[.78rem]"
        type="button"
        onClick={() => void logout().then(() => window.location.assign("/"))}
      >
        Log out
      </button>
    </div>
  );
}
