import type { ReactNode } from "react";
import DailyCaper from "./DailyCaper";
import Wordmark from "./Wordmark";

/**
 * Dead ends (not found, failed loads): the site header's wordmark, today's
 * character and one way forward, so an error still looks like Caper.
 */
export default function StatusPage({
  className,
  eyebrow,
  title,
  children,
}: {
  className?: string;
  eyebrow?: string;
  title?: string;
  children: ReactNode;
}) {
  return (
    <main className={className ? `status-page ${className}` : "status-page"}>
      <header className="site-header">
        <Wordmark />
      </header>
      <section>
        <DailyCaper className="status-page-caper" />
        {eyebrow && <p className="status-page-eyebrow">{eyebrow}</p>}
        {title && <h1>{title}</h1>}
        {children}
      </section>
    </main>
  );
}

/** Router fallback when a page fails to load, instead of the framework's raw error. */
export function RouteError() {
  return (
    <StatusPage eyebrow="Something went wrong" title="Caper didn’t load.">
      <p>Check your connection, then try again.</p>
      <div className="status-page-actions">
        <a href="/">Return home</a>
        <button type="button" className="primary" onClick={() => window.location.reload()}>
          Try again
        </button>
      </div>
    </StatusPage>
  );
}
