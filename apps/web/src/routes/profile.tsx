import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { getAccount, getRememberedAccount, logout, type Account } from "../account/client";
import ProfileForm from "../account/ProfileForm";

export const Route = createFileRoute("/profile")({ component: Profile });

function Profile() {
  const navigate = useNavigate();
  const [account, setAccount] = useState<Account | undefined>(() => getRememberedAccount());

  useEffect(() => {
    if (account) return;
    void getAccount().then((result) => {
      if (!result) return void navigate({ to: "/login", replace: true });
      setAccount(result);
    }).catch(() => void navigate({ to: "/login", replace: true }));
  }, [account, navigate]);

  if (!account) return null;

  return <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
    <section className="w-full max-w-[440px]">
      <a className="wordmark" href="/" aria-label="Caper home">caper<span className="wordmark-dot">.</span></a>
      <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">{account.username ? "YOUR ACCOUNT" : "ONE LAST THING"}</p>
      <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em]">{account.username ? "Make it yours." : "Choose how you show up."}</h1>
      <p className="leading-[1.65] text-content-muted">Your username is unique. Your display name is what people see in conversations.</p>
      <ProfileForm account={account} onSaved={() => navigate({ to: "/spaces" })} />
      <div className="mt-5 flex items-center justify-between text-[.85rem]">
        <a className="text-content underline-offset-4 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" href="/spaces">Go to spaces</a>
        <button className="cursor-pointer border-0 bg-transparent p-0 text-content-muted focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button>
      </div>
    </section>
  </main>;
}
