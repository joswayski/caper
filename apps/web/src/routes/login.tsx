import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { FormEvent, useEffect, useState } from "react";
import { AccountApiError, getAccount, requestEmailCode, verifyEmailCode } from "../account/client";

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
      if (account) void navigate({ to: account.username ? "/spaces" : "/profile", replace: true });
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
      await navigate({ to: account.username ? "/spaces" : "/profile" });
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

  return <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
    <section className="w-full max-w-[440px]">
      <a className="wordmark" href="/" aria-label="Caper home">caper<span className="wordmark-dot">.</span></a>
      <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">WELCOME TO CAPER</p>
      <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em]">{challengeId ? "Check your email." : "Come on in."}</h1>
      {challengeId ? <>
        <p className="leading-[1.65] text-content-muted">Enter the six-character code sent to <strong className="wrap-anywhere text-content">{email.trim()}</strong>. It expires in 10 minutes.</p>
        <form onSubmit={(event) => void verifyCode(event)}>
          <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="code">Sign-in code</label>
          <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 disabled:cursor-not-allowed disabled:opacity-55" id="code" name="code" value={code} onChange={(event) => setCode(event.target.value.toUpperCase().replace(/[^A-HJKMNPQRSTWXYZ2-9]/g, "").slice(0, 6))} autoComplete="one-time-code" autoCapitalize="characters" spellCheck={false} pattern="[A-HJKMNPQRSTWXYZ2-9]{6}" maxLength={6} disabled={attemptsRemaining === 0} required autoFocus />
          {error && <p className="mt-5 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">{error}</p>}
          {attemptsRemaining === 1 && <p className="mt-3 text-[.9rem] leading-[1.5] font-bold" role="status">One attempt left. Check the code carefully.</p>}
          {attemptsRemaining === 0
            ? <button className="mt-7 flex w-full cursor-pointer items-center justify-between gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content disabled:cursor-wait disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="button" disabled={pending} onClick={() => void sendCode()}>{pending ? "Sending…" : <>Email me a new code <span aria-hidden="true">→</span></>}</button>
            : <button className={`mt-7 flex w-full cursor-pointer items-center gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content disabled:cursor-wait disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 ${pending ? "justify-center" : "justify-between"}`} type="submit" disabled={pending || code.length !== 6}>{pending ? <><span className="size-[1em] animate-spin rounded-full border-2 border-current border-r-transparent" aria-hidden="true" />Checking…</> : <>Continue <span aria-hidden="true">→</span></>}</button>}
          <button className="cursor-pointer border-0 bg-transparent py-4 text-[.85rem] text-content-muted focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="button" disabled={pending} onClick={() => { setChallengeId(undefined); setError(undefined); setAttemptsRemaining(undefined); }}>Use a different email</button>
        </form>
      </> : <>
        <p className="leading-[1.65] text-content-muted">Use your email to create an account or return to one. No password needed.</p>
        <form onSubmit={(event) => void requestCode(event)}>
          <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="email">Email address</label>
          <input className="w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 disabled:cursor-not-allowed disabled:opacity-55" id="email" name="email" type="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="email" placeholder="you@example.com" required autoFocus />
          {error && <p className="mt-5 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">{error}</p>}
          <button className="mt-7 flex w-full cursor-pointer items-center justify-between gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content disabled:cursor-wait disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="submit" disabled={pending}>{pending ? "Sending…" : <>Email me a code <span aria-hidden="true">→</span></>}</button>
        </form>
        <small className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">We only send a code when you ask. Prefer to look around first? <a className="text-content underline-offset-3" href="/">Join #general as a guest.</a></small>
      </>}
    </section>
  </main>;
}
