import { useEffect, useState } from "react";
import { getAccount, logout, type Account } from "./client";

export default function AccountNav({ account: initialAccount }: { account?: Account | null }) {
  const [account, setAccount] = useState(initialAccount);

  useEffect(() => {
    if (initialAccount !== undefined) return;

    let current = true;
    void getAccount()
      .then((result) => { if (current) setAccount(result); })
      .catch(() => { if (current) setAccount(null); });
    return () => { current = false; };
  }, [initialAccount]);

  if (account === undefined) return <span className="size-[7px] rounded-full bg-border" aria-label="Checking account" />;
  if (account === null) return <a className="text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 max-[560px]:text-[.78rem]" href="/login">Sign in</a>;

  return <div className="flex items-center gap-4 max-[560px]:gap-2.5">
    <a className="text-sm font-bold text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 max-[560px]:text-[.78rem]" href="/profile">
      {account.username ? `@${account.username}` : "Finish account"}
    </a>
    <button className="cursor-pointer border-0 bg-transparent p-0 text-sm font-bold text-content-muted hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 max-[560px]:text-[.78rem]" type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button>
  </div>;
}
