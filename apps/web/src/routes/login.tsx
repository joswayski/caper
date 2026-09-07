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
      <h1>Conversations aren’t open yet.</h1>
      <p>Caper is live as a preview, but accounts and voice rooms are currently unavailable while we build the next version.</p>
      <a className="account-primary" href="/">Return home <span aria-hidden="true">→</span></a>
      <small>There is no sign-up or sign-in available right now.</small>
    </section>
  </main>;
}
