import { useEffect, useState } from "react";
import AccountNav from "../account/AccountNav";
import type { Account } from "../account/client";
import LiveWindow from "../components/LiveWindow";
import Wordmark from "../components/Wordmark";
import { liveRestScript } from "../components/liveMotion";
import type { GeneralChatHistory } from "../chat/types";
import { detectDownloadPlatform, downloads, type DownloadPlatform } from "../downloads";

const repositoryUrl = "https://github.com/joswayski/caper";
const xUrl = "https://x.com/josevalerio";
const contactEmail = "contact@josevalerio.com";
const rotatingWords = ["people", "friends", "teammates", "coworkers", "family"];

type HomeProps = {
  account: Account | null;
  history: GeneralChatHistory | null;
  initialNow: number;
  latestChanges: readonly LatestChange[];
  downloadPlatform: DownloadPlatform | null;
};

const relativeTimeFormatter = new Intl.RelativeTimeFormat("en", { numeric: "always" });

export default function Home({ account, history, initialNow, latestChanges, downloadPlatform }: HomeProps) {
  const [now, setNow] = useState(initialNow);
  const [open, setOpen] = useState(false);
  const [platform, setPlatform] = useState(downloadPlatform);
  const download = platform ? downloads[platform] : null;

  useEffect(() => {
    // iPadOS can send a Mac user agent; touch points distinguish it after hydration.
    const hints = navigator as Navigator & { userAgentData?: { platform: string; mobile: boolean } };
    setPlatform(detectDownloadPlatform({
      userAgent: navigator.userAgent,
      platform: hints.userAgentData?.platform ?? navigator.platform,
      mobile: hints.userAgentData?.mobile,
      maxTouchPoints: navigator.maxTouchPoints,
    }));
  }, []);

  useEffect(() => {
    setNow(Date.now());
    const interval = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(interval);
  }, []);

  return (
    <main className="page">
      <header className="site-header shell">
        <Wordmark />
        <div className="site-header-actions">
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
          <div className="app-downloads">
            <div className="download-actions">
              {download && platform && <a className="download-button" href={download.url}><PlatformIcon platform={platform} />{download.label}</a>}
              <a className="source-button" href={repositoryUrl} target="_blank" rel="noreferrer"><GitHubIcon />View source</a>
            </div>
            {download ?
              <p><a href={`${repositoryUrl}/releases/tag/native-latest`}>Also available for {platform === "macos" ? "Windows and Linux" : platform === "windows" ? "macOS and Linux" : "macOS and Windows"}</a></p>
              : <p>Available for <a href={`${repositoryUrl}/releases/tag/native-latest`}>macOS, Windows, and Linux</a>.</p>}
          </div>
          <p className="experimental-note">
            Mobile apps are coming soon. Caper may contain bugs or incomplete features. Please give feedback on <a className="feedback-x" href={xUrl} target="_blank" rel="noreferrer" aria-label="Give feedback on X"><XIcon /></a> or by email:
          </p>
          <div className="feedback-email"><CopyEmailButton email={contactEmail} /></div>
          <p className="made-by">Made by <a href={xUrl} target="_blank" rel="noreferrer">Jose Valerio</a></p>
        </div>

        <LiveWindow account={account} history={history} active={open} onActiveChange={setOpen} />
      </section>
      {/* Runs during parsing, once the hero it measures is complete. */}
      <script dangerouslySetInnerHTML={{ __html: liveRestScript }} suppressHydrationWarning />

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

// Platform logo paths shared with Captures' homepage download buttons.
function PlatformIcon({ platform }: { platform: DownloadPlatform }) {
  const path = platform === "macos"
    ? "M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"
    : platform === "windows"
      ? "M3 5.15 11.15 4v7.35H3V5.15Zm9.15-1.45L21 2.4v8.95h-8.85V3.7ZM3 12.85h8.15V20.2L3 19.05v-6.2Zm9.15 0H21v8.95l-8.85-1.45v-7.5Z"
      : "M12 2.2c-2.05 0-3.7 1.7-3.7 4 0 .72.18 1.38.5 1.94-2.28 1.32-3.85 3.55-4.38 6.1-.52 2.5.05 4.28 1.55 5.15-.62.42-1.02 1.1-1.02 1.88 0 1.42 1.48 2.18 3.05 2.52 1.42.3 3.12.36 5 .36s3.58-.06 5-.36c1.57-.34 3.05-1.1 3.05-2.52 0-.78-.4-1.46-1.02-1.88 1.5-.87 2.07-2.65 1.55-5.15-.53-2.55-2.1-4.78-4.38-6.1.32-.56.5-1.22.5-1.94 0-2.3-1.65-4-3.7-4ZM10.2 5.55c.45-.3.95.05.88.58-.06.46-.58.72-1.02.45-.44-.26-.42-.82.14-1.03Zm3.8 0c.56.21.58.77.14 1.03-.44.27-.96.01-1.02-.45-.07-.53.43-.88.88-.58ZM9.35 16.85c.9.5 1.75.78 2.65.78s1.75-.28 2.65-.78c.28-.16.6.02.6.34 0 .78-1.05 1.5-3.25 1.5s-3.25-.72-3.25-1.5c0-.32.32-.5.6-.34Z";
  return <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path fillRule="evenodd" d={path} /></svg>;
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
