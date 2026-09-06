const repositoryUrl = "https://github.com/joswayski/caper";

export default function Home() {
  return (
    <main className="page">
      <header className="site-header shell">
        <a className="wordmark" href="/" aria-label="Caper home">caper</a>
        <span className="header-status"><i /> Coming soon</span>
      </header>

      <section className="hero shell">
        <div className="hero-copy">
          <h1>A place for <span>your people</span></h1>
          <p>
            Chat, call, and share in a space that feels like yours. For friends,
            teams, and everything in between.
          </p>
          <a className="github-button" href={repositoryUrl} target="_blank" rel="noreferrer">
            Follow on GitHub <span aria-hidden="true">↗</span>
          </a>
        </div>

        <div className="people-collage" aria-hidden="true">
          <div className="person-card person-card-terracotta"><i /><span /></div>
          <div className="person-card person-card-cream"><i /><span /></div>
          <div className="person-card person-card-green"><i /><span /></div>
          <div className="person-card person-card-charcoal"><i /><span /></div>
        </div>
      </section>
    </main>
  );
}
