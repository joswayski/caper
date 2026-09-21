import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { FormEvent, useEffect, useState } from "react";
import { AccountApiError, getAccount, getRememberedAccount, logout, updateProfile, type Account } from "../account/client";

export const Route = createFileRoute("/profile")({ component: Profile });

function Profile() {
  const navigate = useNavigate();
  const [account, setAccount] = useState<Account | undefined>(() => getRememberedAccount());
  const [username, setUsername] = useState(() => getRememberedAccount()?.username ?? "");
  const [displayName, setDisplayName] = useState(() => getRememberedAccount()?.displayName ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    if (account) return;
    void getAccount().then((result) => {
      if (!result) return void navigate({ to: "/login", replace: true });
      setAccount(result);
      setUsername(result.username ?? "");
      setDisplayName(result.displayName ?? "");
    }).catch(() => void navigate({ to: "/login", replace: true }));
  }, [account, navigate]);

  async function save(event: FormEvent) {
    event.preventDefault();
    setPending(true);
    setError(undefined);
    try {
      const updated = await updateProfile(username, displayName);
      setAccount(updated);
      await navigate({ to: "/live" });
    } catch (saveError) {
      if (saveError instanceof AccountApiError && saveError.status === 409) setError("That username is already taken.");
      else if (saveError instanceof AccountApiError && saveError.status === 400) setError("Check the username and display name requirements.");
      else setError("Your profile could not be saved. Please try again.");
    } finally {
      setPending(false);
    }
  }

  if (!account) return null;

  return <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
    <section className="w-full max-w-[440px]">
      <a className="wordmark" href="/" aria-label="Caper home">caper<span className="wordmark-dot">.</span></a>
      <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">{account.username ? "YOUR ACCOUNT" : "ONE LAST THING"}</p>
      <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em]">{account.username ? "Make it yours." : "Choose how you show up."}</h1>
      <p className="leading-[1.65] text-content-muted">Your username is unique. Your display name is what people see in conversations.</p>
      <form onSubmit={(event) => void save(event)}>
        <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="username">Username</label>
        <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" id="username" name="username" value={username} onChange={(event) => setUsername(event.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "").slice(0, 32))} autoComplete="username" minLength={3} maxLength={32} pattern="[a-z0-9_]{3,32}" required />
        <small className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">3-32 lowercase letters, numbers, or underscores.</small>
        <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="display-name">Display name</label>
        <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" id="display-name" name="displayName" value={displayName} onChange={(event) => setDisplayName(event.target.value.slice(0, 64))} autoComplete="name" maxLength={64} required />
        <small className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">Shown to other people. It does not need to be unique.</small>
        {error && <p className="mt-5 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">{error}</p>}
        <button className="mt-7 flex w-full cursor-pointer items-center justify-between gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content disabled:cursor-wait disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="submit" disabled={pending}>{pending ? "Saving…" : <>{account.username ? "Save profile" : "Finish account"} <span aria-hidden="true">→</span></>}</button>
      </form>
      <div className="mt-5 flex items-center justify-between text-[.85rem]">
        <a className="text-content underline-offset-4 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" href="/live">Go to General</a>
        <button className="cursor-pointer border-0 bg-transparent p-0 text-content-muted focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button>
      </div>
    </section>
  </main>;
}
