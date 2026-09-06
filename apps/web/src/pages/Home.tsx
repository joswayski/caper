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
