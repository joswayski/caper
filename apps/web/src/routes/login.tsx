import { createFileRoute } from "@tanstack/react-router";
import "../pages/account.css";

export const Route = createFileRoute("/login")({
  validateSearch: (search: Record<string, unknown>): { error?: "auth" } => ({ error: search.error === "auth" ? "auth" : undefined }),
  component: Login,
});

function Login() {
  const { error } = Route.useSearch();
  return <main className="account-page">
    <section className="account-card">
      <span className="wordmark">caper<span className="account-dot">.</span></span>
      <p className="account-eyebrow">GOOD COMPANY STARTS HERE</p>
      <h1>Your people.<br />Your place.</h1>
      <p>Sign in or create an account to join the conversation. We’ll send a one-time code to your email—no password needed.</p>
      {error && <p className="account-error" role="alert">Sign-in could not be completed. Please try again.</p>}
      <a className="account-primary" href="/api/auth/sign-in">Continue with email <span aria-hidden="true">→</span></a>
      <small>Secure sign-in provided by WorkOS.</small>
    </section>
  </main>;
}
