import { useEffect, useState } from "react";
import LowPolyChat from "../components/LowPolyChat";

const repositoryUrl = "https://github.com/joswayski/caper";
const rotatingWords = ["people", "friends", "teammates", "coworkers", "family"];

export default function Home() {
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
            Chat, call, and share in a space that feels like yours. For friends,
            teams, and everything in between.
          </p>
          <div className="hero-actions">
            <span className="coming-soon"><i /> Coming soon</span>
            <span className="action-separator" aria-hidden="true">·</span>
            <a className="github-button" href={repositoryUrl} target="_blank" rel="noreferrer">
              Follow on GitHub <span aria-hidden="true">↗</span>
            </a>
          </div>
        </div>

        <LowPolyChat />
      </section>
    </main>
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
