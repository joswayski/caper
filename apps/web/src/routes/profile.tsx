import { createFileRoute } from "@tanstack/react-router";
import { FormEvent, useEffect, useState } from "react";
import { AccountApiError, getAccount, logout, updateProfile, type Account } from "../account/client";
import "../pages/account.css";

export const Route = createFileRoute("/profile")({ component: Profile });

function Profile() {
  const [account, setAccount] = useState<Account>();
  const [username, setUsername] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    void getAccount().then((result) => {
      if (!result) return window.location.replace("/login");
      setAccount(result);
      setUsername(result.username ?? "");
      setDisplayName(result.displayName ?? "");
    }).catch(() => setError("Your account could not be loaded. Please try again."));
  }, []);

  async function save(event: FormEvent) {
    event.preventDefault();
    setPending(true);
    setError(undefined);
    try {
      const updated = await updateProfile(username, displayName);
      setAccount(updated);
      window.location.assign("/live");
    } catch (saveError) {
      if (saveError instanceof AccountApiError && saveError.status === 409) setError("That username is already taken.");
      else if (saveError instanceof AccountApiError && saveError.status === 400) setError("Check the username and display name requirements.");
      else setError("Your profile could not be saved. Please try again.");
    } finally {
      setPending(false);
    }
  }

  if (!account) return <main className="account-page"><p role="status">{error ?? "Loading your account…"}</p></main>;

  return <main className="account-page">
    <section className="account-card">
      <a className="wordmark" href="/" aria-label="Caper home">caper<span className="account-dot">.</span></a>
      <p className="account-eyebrow">{account.username ? "YOUR ACCOUNT" : "ONE LAST THING"}</p>
      <h1>{account.username ? "Make it yours." : "Choose how you show up."}</h1>
      <p>Your username is unique. Your display name is what people see in conversations.</p>
      <form onSubmit={(event) => void save(event)}>
        <label htmlFor="username">Username</label>
        <input id="username" name="username" value={username} onChange={(event) => setUsername(event.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "").slice(0, 32))} autoComplete="username" minLength={3} maxLength={32} pattern="[a-z0-9_]{3,32}" required />
        <small>3–32 lowercase letters, numbers, or underscores.</small>
        <label htmlFor="display-name">Display name</label>
        <input id="display-name" name="displayName" value={displayName} onChange={(event) => setDisplayName(event.target.value.slice(0, 64))} autoComplete="name" maxLength={64} required />
        <small>Shown to other people. It does not need to be unique.</small>
        {error && <p className="account-error" role="alert">{error}</p>}
        <button className="account-primary" type="submit" disabled={pending}>{pending ? "Saving…" : <>{account.username ? "Save profile" : "Finish account"} <span aria-hidden="true">→</span></>}</button>
      </form>
      <div className="account-card-actions">
        <a href="/live">Go to General</a>
        <button type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button>
      </div>
    </section>
  </main>;
}
