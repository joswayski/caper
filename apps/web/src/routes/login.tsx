import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { FormEvent, useEffect, useState } from "react";
import { AccountApiError, getAccount, requestEmailCode, verifyEmailCode } from "../account/client";
import "../pages/account.css";

export const Route = createFileRoute("/login")({ component: Login });

function loginError(error: unknown) {
  if (error instanceof AccountApiError) {
    if (error.status === 401 && error.attemptsRemaining === 0) return "That code can no longer be used. Request a new one.";
    if (error.status === 401) return "That code is incorrect or expired. Request a new one if needed.";
    if (error.status === 400) return "Enter a valid email address.";
    if (error.status === 503) return "Sign-in is temporarily unavailable. Please try again later.";
  }
  return "Something went wrong. Please try again.";
}

function Login() {
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [challengeId, setChallengeId] = useState<string>();
  const [code, setCode] = useState("");
  const [attemptsRemaining, setAttemptsRemaining] = useState<number>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    void getAccount().then((account) => {
      if (account) void navigate({ to: account.username ? "/live" : "/profile", replace: true });
    }).catch(() => undefined);
  }, [navigate]);

  async function sendCode() {
    setPending(true);
    setError(undefined);
    setAttemptsRemaining(undefined);
    try {
      const result = await requestEmailCode(email);
      setChallengeId(result.challengeId);
      setCode("");
    } catch (requestError) {
      setError(loginError(requestError));
    } finally {
      setPending(false);
    }
  }

  function requestCode(event: FormEvent) {
    event.preventDefault();
    void sendCode();
  }

  async function verifyCode(event: FormEvent) {
    event.preventDefault();
    if (!challengeId) return;
    setPending(true);
    setError(undefined);
    try {
      const account = await verifyEmailCode(challengeId, code);
      await navigate({ to: account.username ? "/live" : "/profile" });
    } catch (verifyError) {
      if (verifyError instanceof AccountApiError) {
        setAttemptsRemaining(verifyError.attemptsRemaining);
        if (verifyError.attemptsRemaining === 0) setCode("");
      }
      setError(loginError(verifyError));
    } finally {
      setPending(false);
    }
  }

  return <main className="account-page">
    <section className="account-card">
      <a className="wordmark" href="/" aria-label="Caper home">caper<span className="wordmark-dot">.</span></a>
      <p className="account-eyebrow">WELCOME TO CAPER</p>
      <h1>{challengeId ? "Check your email." : "Come on in."}</h1>
      {challengeId ? <>
        <p>Enter the six-character code sent to <strong>{email.trim()}</strong>. It expires in 10 minutes.</p>
        <form onSubmit={(event) => void verifyCode(event)}>
          <label htmlFor="code">Sign-in code</label>
          <input id="code" name="code" value={code} onChange={(event) => setCode(event.target.value.toUpperCase().replace(/[^A-HJKMNPQRSTWXYZ2-9]/g, "").slice(0, 6))} autoComplete="one-time-code" autoCapitalize="characters" spellCheck={false} pattern="[A-HJKMNPQRSTWXYZ2-9]{6}" maxLength={6} disabled={attemptsRemaining === 0} required autoFocus />
          {error && <p className="account-error" role="alert">{error}</p>}
          {attemptsRemaining === 1 && <p className="account-warning" role="status">One attempt left. Check the code carefully.</p>}
          {attemptsRemaining === 0
            ? <button className="account-primary" type="button" disabled={pending} onClick={() => void sendCode()}>{pending ? "Sending…" : <>Email me a new code <span aria-hidden="true">→</span></>}</button>
            : <button className={`account-primary${pending ? " account-primary-loading" : ""}`} type="submit" disabled={pending || code.length !== 6}>{pending ? <><span className="account-spinner" aria-hidden="true" />Checking…</> : <>Continue <span aria-hidden="true">→</span></>}</button>}
          <button className="account-secondary" type="button" disabled={pending} onClick={() => { setChallengeId(undefined); setError(undefined); setAttemptsRemaining(undefined); }}>Use a different email</button>
        </form>
      </> : <>
        <p>Use your email to create an account or return to one. No password needed.</p>
        <form onSubmit={(event) => void requestCode(event)}>
          <label htmlFor="email">Email address</label>
          <input id="email" name="email" type="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="email" placeholder="you@example.com" required autoFocus />
          {error && <p className="account-error" role="alert">{error}</p>}
          <button className="account-primary" type="submit" disabled={pending}>{pending ? "Sending…" : <>Email me a code <span aria-hidden="true">→</span></>}</button>
        </form>
        <small>We only send a code when you ask. Prefer to look around first? <a href="/live">Join General as a guest.</a></small>
      </>}
    </section>
  </main>;
}
