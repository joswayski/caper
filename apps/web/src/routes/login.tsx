import { createFileRoute } from "@tanstack/react-router";
import "../pages/account.css";

export const Route = createFileRoute("/login")({
  component: Login,
});

function Login() {
  return <main className="account-page">
    <section className="account-card">
      <span className="wordmark">caper<span className="account-dot">.</span></span>
      <p className="account-eyebrow">EARLY DAYS</p>
      <h1>No account needed.</h1>
      <p>Try the public General voice channel with a guest name while we build the next version.</p>
      <a className="account-primary" href="/live">Try voice <span aria-hidden="true">→</span></a>
      <small>There is no sign-up or sign-in available right now.</small>
    </section>
  </main>;
}
