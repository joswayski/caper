import { useState, type FormEvent } from "react";
import { AccountApiError, updateProfile, type Account } from "./client";

export default function ProfileForm({ account, onSaved }: { account: Account; onSaved: (account: Account) => void | Promise<void> }) {
  const [username, setUsername] = useState(account.username ?? "");
  const [displayName, setDisplayName] = useState(account.displayName ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  async function save(event: FormEvent) {
    event.preventDefault();
    setPending(true);
    setError(undefined);
    try {
      await onSaved(await updateProfile(username, displayName));
    } catch (saveError) {
      if (saveError instanceof AccountApiError && saveError.status === 409) setError("That username is already taken.");
      else if (saveError instanceof AccountApiError && saveError.status === 400) setError("Check the username and display name requirements.");
      else setError("Your profile could not be saved. Please try again.");
    } finally {
      setPending(false);
    }
  }

  return <form onSubmit={(event) => void save(event)}>
    <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="username">Username</label>
    <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" id="username" name="username" value={username} onChange={(event) => setUsername(event.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "").slice(0, 32))} autoComplete="username" minLength={3} maxLength={32} pattern="[a-z0-9_]{3,32}" required />
    <small className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">3-32 lowercase letters, numbers, or underscores.</small>
    <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="display-name">Display name</label>
    <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" id="display-name" name="displayName" value={displayName} onChange={(event) => setDisplayName(event.target.value.slice(0, 64))} autoComplete="name" maxLength={64} required />
    <small className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">Shown to other people. It does not need to be unique.</small>
    {error && <p className="mt-5 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">{error}</p>}
    <button className="mt-7 flex w-full cursor-pointer items-center justify-between gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content disabled:cursor-wait disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="submit" disabled={pending}>{pending ? "Saving…" : <>{account.username ? "Save profile" : "Finish account"} <span aria-hidden="true">→</span></>}</button>
  </form>;
}
