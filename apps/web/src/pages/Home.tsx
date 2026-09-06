import { useEffect, useState } from "react";
import LowPolyChat from "../components/LowPolyChat";

const repositoryUrl = "https://github.com/joswayski/caper";
const xUrl = "https://x.com/josevalerio";
const contactEmail = "contact@josevalerio.com";
const rotatingWords = ["people", "friends", "teammates", "coworkers", "family"];

export default function Home() {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const interval = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(interval);
  }, []);

  return (
    <main className="page">
      <header className="site-header shell">
        <a className="wordmark" href="/" aria-label="Caper home">caper</a>
      </header>

      <section className="hero shell">
        <div className="hero-copy">
          <h1 aria-label="A place for your people">
            <span className="hero-title-line">A place for</span>
            <span className="hero-title-line">
              your{" "}
              <Rolodex words={rotatingWords} />
            </span>
          </h1>
          <p>
            Chat with anyone, about anything.
          </p>
          <p className="made-by">
            A place for conversations by <a href={xUrl} target="_blank" rel="noreferrer">Jose Valerio</a>.
          </p>
          <p className="experimental-label">(experimental)</p>
          <div className="hero-actions">
            <a className="github-button" href="/live">Open voice channel</a>
            <span className="action-separator" aria-hidden="true">·</span>
            <a className="coming-soon" href={repositoryUrl} target="_blank" rel="noreferrer">
              Follow on GitHub <span aria-hidden="true">↗</span>
            </a>
          </div>
          <p className="experimental-note">
            Caper may contain bugs or incomplete features. A desktop app is coming soon. Please give feedback on <a className="feedback-x" href={xUrl} target="_blank" rel="noreferrer" aria-label="Give feedback on X"><XIcon /></a>, or <CopyEmailButton email={contactEmail} />.
          </p>
        </div>

        <LowPolyChat />
      </section>

      <section className="latest-changes shell" aria-labelledby="latest-changes-heading">
        <h2 id="latest-changes-heading">Latest changes</h2>
        {__LATEST_CHANGES__.length > 0 ? (
          <ol>
            {__LATEST_CHANGES__.map((change) => (
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
  const seconds = Math.max(0, Math.round((new Date(committedAt).getTime() - now) / 1_000));
  const formatter = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

  if (seconds > -60) return formatter.format(seconds, "second");
  const minutes = Math.round(seconds / 60);
  if (minutes > -60) return formatter.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (hours > -24) return formatter.format(hours, "hour");
  const days = Math.round(hours / 24);
  if (days > -30) return formatter.format(days, "day");
  const months = Math.round(days / 30);
  if (months > -12) return formatter.format(months, "month");
  return formatter.format(Math.round(months / 12), "year");
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
