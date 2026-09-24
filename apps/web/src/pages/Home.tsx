import { useEffect, useState } from "react";
import AccountNav from "../account/AccountNav";
import type { Account } from "../account/client";
import LiveWindow from "../components/LiveWindow";
import type { GeneralChatHistory } from "../chat/types";

const repositoryUrl = "https://github.com/joswayski/caper";
const xUrl = "https://x.com/josevalerio";
const contactEmail = "contact@josevalerio.com";
const rotatingWords = ["people", "friends", "teammates", "coworkers", "family"];

type HomeProps = {
  account: Account | null;
  history: GeneralChatHistory | null;
  initialNow: number;
  latestChanges: readonly LatestChange[];
};

const relativeTimeFormatter = new Intl.RelativeTimeFormat("en", { numeric: "always" });

export default function Home({ account, history, initialNow, latestChanges }: HomeProps) {
  const [now, setNow] = useState(initialNow);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setNow(Date.now());
    const interval = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(interval);
  }, []);

  return (
    <main className="page">
      <header className="site-header shell">
        <a className="wordmark" href="/" aria-label="Caper home">caper<span className="wordmark-dot">.</span></a>
        <div className="site-header-actions">
          <a className="header-github" href={repositoryUrl} target="_blank" rel="noreferrer" aria-label="Caper on GitHub"><GitHubIcon /></a>
          <AccountNav account={account} />
        </div>
      </header>

      <section className="hero shell" data-live-bounds data-live-open={open ? "" : undefined}>
        <div className="hero-copy" inert={open}>
          <h1 aria-label="A place for your people">
            <span className="hero-title-line">A place for</span>
            <span className="hero-title-line">
              your{" "}
              <Rolodex words={rotatingWords} />
            </span>
          </h1>
          <p className="hero-lede">Chat with anyone, about anything.</p>
          <p className="made-by">
            Created by <a href={xUrl} target="_blank" rel="noreferrer">Jose Valerio</a> · <a href={repositoryUrl} target="_blank" rel="noreferrer">Follow on GitHub</a>
          </p>
          <p className="experimental-note">
            Caper may contain bugs or incomplete features. A desktop app is coming soon. Please give feedback on <a className="feedback-x" href={xUrl} target="_blank" rel="noreferrer" aria-label="Give feedback on X"><XIcon /></a> or by email:
          </p>
          <div className="feedback-email"><CopyEmailButton email={contactEmail} /></div>
        </div>

        <LiveWindow account={account} history={history} active={open} onActiveChange={setOpen} />
      </section>

      <section className="latest-changes shell" aria-labelledby="latest-changes-heading">
        <h2 id="latest-changes-heading">Latest changes</h2>
        {latestChanges.length > 0 ? (
          <ol>
            {latestChanges.map((change) => (
              <li key={change.sha}>
                <a href={change.url} target="_blank" rel="noreferrer">{change.title}</a>
                <time dateTime={change.committedAt}>{formatRelativeTime(change.committedAt, now)}</time>
              </li>
            ))}
          </ol>
        ) : (
          <p className="latest-changes-empty">Recent work will appear here after the next build.</p>
        )}
      </section>
    </main>
  );
}

function formatRelativeTime(committedAt: string, now: number) {
  const divisions = [
    { amount: 60, unit: "second" },
    { amount: 60, unit: "minute" },
    { amount: 24, unit: "hour" },
    { amount: 7, unit: "day" },
    { amount: 4.345, unit: "week" },
    { amount: 12, unit: "month" },
    { amount: Number.POSITIVE_INFINITY, unit: "year" },
  ] as const;

  let duration = (new Date(committedAt).getTime() - now) / 1_000;
  for (const division of divisions) {
    if (Math.abs(duration) < division.amount) {
      return relativeTimeFormatter.format(Math.round(duration), division.unit);
    }
    duration /= division.amount;
  }

  return relativeTimeFormatter.format(0, "second");
}

function GitHubIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" fill="currentColor">
      <path d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.56-.29-5.25-1.28-5.25-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a10.9 10.9 0 0 1 5.76 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.84 1.19 3.1 0 4.42-2.7 5.39-5.27 5.68.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5Z" />
    </svg>
  );
}

function XIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" fill="currentColor">
      <path d="M18.9 2.25h3.68l-8.04 9.19L24 21.75h-7.4l-5.8-7.58-6.63 7.58H.48l8.6-9.83L0 2.25h7.59l5.24 6.93 6.07-6.93Zm-1.29 17.29h2.04L6.48 4.34H4.29L17.61 19.54Z" />
    </svg>
  );
}

function CopyEmailButton({ email }: { email: string }) {
  const [copied, setCopied] = useState(false);

  async function copyEmail() {
    try {
      await navigator.clipboard.writeText(email);
    } catch {
      const input = document.createElement("textarea");
      input.value = email;
      input.style.position = "fixed";
      input.style.opacity = "0";
      document.body.append(input);
      input.select();
      document.execCommand("copy");
      input.remove();
    }

    setCopied(true);
    window.setTimeout(() => setCopied(false), 1_800);
  }

  return (
    <button className="copy-email" type="button" onClick={() => void copyEmail()}>
      <span>{email}</span>
      <small>{copied ? "copied!" : "click to copy"}</small>
    </button>
  );
}

function Rolodex({ words }: { words: readonly string[] }) {
  const [index, setIndex] = useState(0);

  useEffect(() => {
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (reducedMotion.matches) return;

    const interval = window.setInterval(() => {
      setIndex((current) => (current + 1) % words.length);
    }, 2400);

    return () => window.clearInterval(interval);
  }, [words.length]);

  const longestWord = words.reduce((longest, word) =>
    word.length > longest.length ? word : longest,
  );
  const previousIndex = (index + words.length - 1) % words.length;

  return (
    <span className="rolodex" aria-hidden="true">
      <span className="rolodex-sizer">{longestWord}</span>
      {words.map((word, wordIndex) => {
        const state = wordIndex === index
          ? "current"
          : wordIndex === previousIndex
            ? "exit"
            : "idle";

        return (
          <span key={word} className="rolodex-word" data-state={state}>
            {word}
          </span>
        );
      })}
    </span>
  );
}
