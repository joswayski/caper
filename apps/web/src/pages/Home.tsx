const repositoryUrl = "https://github.com/joswayski/caper";

export default function Home() {
  return (
    <main className="page">
      <header className="site-header shell">
        <a className="wordmark" href="/" aria-label="Caper home">caper</a>
      </header>

      <section className="hero shell">
        <h1>A place for your people</h1>
        <p>
          Chat, call, and share in a space that feels like yours. For friends,
          teams, and everything in between.
        </p>
        <div className="availability">
          <span>Coming soon</span>
          <span className="separator" aria-hidden="true">·</span>
          <a href={repositoryUrl} target="_blank" rel="noreferrer">
            Follow on GitHub <span aria-hidden="true">↗</span>
          </a>
        </div>
      </section>

      <footer className="site-footer shell">
        <span>caper</span>
        <span>Open source · Apache 2.0</span>
      </footer>
    </main>
  );
}
