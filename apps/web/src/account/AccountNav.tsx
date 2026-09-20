import { useEffect, useState } from "react";
import { getAccount, logout, type Account } from "./client";
import "./account-nav.css";

export default function AccountNav() {
  const [account, setAccount] = useState<Account | null>();

  useEffect(() => {
    let current = true;
    void getAccount()
      .then((result) => { if (current) setAccount(result); })
      .catch(() => { if (current) setAccount(null); });
    return () => { current = false; };
  }, []);

  if (account === undefined) return <span className="account-nav-loading" aria-label="Checking account" />;
  if (account === null) return <a className="account-nav-link" href="/login">Sign in</a>;

  return <div className="account-nav-menu">
    <a className="account-nav-link" href="/profile">
      {account.username ? `@${account.username}` : "Finish account"}
    </a>
    <button type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button>
  </div>;
}
