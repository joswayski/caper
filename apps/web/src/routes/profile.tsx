import { createFileRoute, useRouter } from "@tanstack/react-router";
import { useAuth } from "@workos/authkit-tanstack-react-start/client";
import { useState } from "react";
import { loadAccount } from "../server/account";
import "../pages/account.css";

export const Route = createFileRoute("/profile")({ loader: () => loadAccount(), component: Profile });

function Profile() {
  const account = Route.useLoaderData();
  const router = useRouter();
  const { signOut } = useAuth();
  const [username, setUsername] = useState(account.username ?? "");
  const [displayName, setDisplayName] = useState(account.displayName ?? "");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  return <main className="account-page"><section className="account-card">
    <span className="wordmark">caper<span className="account-dot">.</span></span>
    <p className="account-eyebrow">{account.username ? "YOUR ACCOUNT" : "MAKE YOURSELF AT HOME"}</p>
    <h1>{account.username ? "Your profile." : "What should we call you?"}</h1>
    <p>Your username is unique. Your display name is how you’ll appear in conversations.</p>
    <form onSubmit={async (event) => {
      event.preventDefault(); setSaving(true); setError("");
      try {
        const response = await fetch("/api/account/profile", {
          method: "POST", headers: { "content-type": "application/json" },
          body: JSON.stringify({ username, displayName }),
        });
        if (response.status === 401) { window.location.assign("/api/auth/sign-in"); return; }
        if (!response.ok) {
          setError(response.status === 409 ? "That username is taken. Try another." : "Your profile couldn’t be saved. Check your details and try again.");
          return;
        }
        await router.invalidate();
        window.location.assign("/live");
      } catch { setError("We couldn’t connect. Please try again."); }
      finally { setSaving(false); }
    }}>
      <label htmlFor="username">Username</label>
      <input id="username" name="username" autoComplete="username" value={username} onChange={e => setUsername(e.target.value.toLowerCase())} required minLength={3} maxLength={32} pattern="[a-z0-9_]{3,32}" aria-describedby="username-help" />
      <small id="username-help">3–32 letters, numbers, or underscores.</small>
      <label htmlFor="display-name">Display name</label>
      <input id="display-name" name="displayName" autoComplete="nickname" value={displayName} onChange={e => setDisplayName(e.target.value)} required maxLength={128} />
      <small>Up to 64 characters. This doesn’t need to be unique.</small>
      {error && <p className="account-error" role="alert">{error}</p>}
      <button className="account-primary" disabled={saving}>{saving ? "Saving…" : "Save and continue"}</button>
    </form>
    <button className="account-secondary" onClick={() => signOut({ returnTo: "/login" })}>Sign out</button>
  </section></main>;
}
