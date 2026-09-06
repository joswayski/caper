import LowPolyChat from "../components/LowPolyChat";

const repositoryUrl = "https://github.com/joswayski/caper";

export default function Home() {
  return (
    <main className="page">
      <header className="site-header shell">
        <a className="wordmark" href="/" aria-label="Caper home">caper</a>
      </header>

      <section className="hero shell">
        <div className="hero-copy">
          <h1>A place for <span>your people</span></h1>
          <p>
            Drop into a public voice channel and hang out.
            No accounts, no invites. Just good company.
          </p>
          <div className="hero-actions">
            <a className="github-button" href="/live">Open voice channel</a>
            <span className="action-separator" aria-hidden="true">·</span>
            <a className="coming-soon" href={repositoryUrl} target="_blank" rel="noreferrer">
              Follow on GitHub <span aria-hidden="true">↗</span>
            </a>
          </div>
        </div>

        <LowPolyChat />
      </section>
    </main>
  );
}
