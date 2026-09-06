const repositoryUrl = "https://github.com/joswayski/caper";

function ArrowIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="M4 10h11m-4-4 4 4-4 4" />
    </svg>
  );
}

function GithubIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.87c-2.78.61-3.37-1.18-3.37-1.18-.45-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.61.07-.61 1 .07 1.53 1.03 1.53 1.03.9 1.53 2.35 1.09 2.92.83.09-.65.35-1.09.64-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.64 0 0 .84-.27 2.75 1.02A9.6 9.6 0 0 1 12 6.82a9.6 9.6 0 0 1 2.5.34c1.91-1.29 2.75-1.02 2.75-1.02.55 1.37.2 2.39.1 2.64.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.86V21c0 .27.18.58.69.48A10 10 0 0 0 12 2Z" />
    </svg>
  );
}

export default function Home() {
  return (
    <main>
      <nav className="nav shell" aria-label="Primary navigation">
        <a className="wordmark" href="#top" aria-label="Caper home">
          caper<span />
        </a>
        <div className="nav-links">
          <a href="#direction">Direction</a>
          <a href="#principles">Principles</a>
          <a className="nav-github" href={repositoryUrl} target="_blank" rel="noreferrer">
            GitHub <ArrowIcon />
          </a>
        </div>
      </nav>

      <section className="hero shell" id="top">
        <div className="hero-copy">
          <p className="eyebrow"><span>01</span> A LITTLE CHARACTER. A USEFUL PRESENCE.</p>
          <h1>Good company.<br />Great conversations.</h1>
          <p className="hero-lede">
            Messages, calls, and shared moments—together in one calm space.
            Caper is an open-source communication app taking its first steps.
          </p>
          <div className="hero-actions">
            <a className="button button-primary" href={repositoryUrl} target="_blank" rel="noreferrer">
              Follow the build <ArrowIcon />
            </a>
            <a className="button button-secondary" href="#direction">
              See the direction
            </a>
          </div>
          <div className="build-note">
            <span className="status-dot" />
            <div><strong>Early days</strong><small>Designed in public. Built with care.</small></div>
          </div>
        </div>

        <div className="mascot-stage">
          <div className="stage-label">Meet Caper</div>
          <img src="/brand/caper-mascot.webp" alt="A friendly green caper character waving" width="752" height="648" />
          <div className="mascot-caption">
            <span>Recognizable.</span><span>Expressive.</span><span>Useful.</span>
          </div>
        </div>
      </section>

      <section className="direction shell" id="direction">
        <div className="section-heading">
          <p className="eyebrow"><span>02</span> PRODUCT DIRECTION</p>
          <h2>Everything worth sharing.<br />Nothing in the way.</h2>
          <p>Caper is being shaped as a focused home for the conversations that move work—and friendships—forward.</p>
        </div>
        <figure className="product-frame">
          <div className="window-bar"><i /><i /><i /><span>caper / product direction</span></div>
          <img src="/brand/caper-product-direction.webp" alt="Product direction showing Caper conversations, calls, and threads" width="1280" height="853" loading="lazy" />
          <figcaption>Concept artwork · The interface will evolve as Caper takes shape.</figcaption>
        </figure>
      </section>

      <section className="principles shell" id="principles">
        <div className="principle-intro">
          <p className="eyebrow"><span>03</span> BUILT AROUND PEOPLE</p>
          <h2>Conversation should feel natural.</h2>
        </div>
        <div className="principle-grid">
          <article><span>01</span><h3>Clear by default</h3><p>Quiet surfaces, useful hierarchy, and fewer things competing for attention.</p></article>
          <article><span>02</span><h3>Realtime when it matters</h3><p>Move from a message to voice or screen sharing without losing the thread.</p></article>
          <article><span>03</span><h3>Open as we build</h3><p>Follow the decisions, experiments, and implementation in the public repository.</p></article>
        </div>
      </section>

      <section className="closing shell">
        <div>
          <p className="eyebrow"><span>04</span> COME ALONG</p>
          <h2>A little work.<br />A little everything.</h2>
        </div>
        <a className="button button-primary" href={repositoryUrl} target="_blank" rel="noreferrer">
          <GithubIcon /> View Caper on GitHub
        </a>
      </section>

      <footer className="footer shell">
        <a className="wordmark wordmark-small" href="#top">caper<span /></a>
        <p>Good company. Great conversations.</p>
        <p>Open source · Apache 2.0</p>
      </footer>
    </main>
  );
}
