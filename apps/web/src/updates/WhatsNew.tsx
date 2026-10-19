import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ExternalLink, X } from "lucide-react";
import {
  loadReleaseHighlights,
  releaseHighlightDays,
  sourceRevisionValid,
  unreadReleaseHighlights,
  type ReleaseHighlights,
} from "./whats-new";
import "./whats-new.css";

const storageKey = "caper:whats-new:web:v1";

export default function WhatsNew({
  ready,
  blocked,
  manualOpen,
  onManualClose,
}: {
  ready: boolean;
  blocked: boolean;
  manualOpen: boolean;
  onManualClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const acknowledged = useRef<number | undefined>(undefined);
  const initialized = useRef(false);
  const lastAttempt = useRef(0);
  const inFlight = useRef(false);
  const alive = useRef(true);
  const [feed, setFeed] = useState<ReleaseHighlights>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [automaticOpen, setAutomaticOpen] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const open = manualOpen || automaticOpen;
  const revision = __SOURCE_REVISION__;
  const save = useCallback((sequence: number) => {
    acknowledged.current = Math.max(sequence, acknowledged.current ?? 0);
    try {
      localStorage.setItem(storageKey, String(acknowledged.current));
    } catch {
      /* Optional local preference. */
    }
  }, []);

  const load = useCallback(
    async (force = false) => {
      if (
        feed ||
        !sourceRevisionValid(revision) ||
        inFlight.current ||
        (!force && Date.now() - lastAttempt.current < 60_000)
      )
        return;
      if (!initialized.current) {
        initialized.current = true;
        try {
          const stored = Number(localStorage.getItem(storageKey));
          if (Number.isSafeInteger(stored) && stored > 0) acknowledged.current = stored;
        } catch {
          /* Keep an in-memory acknowledgement when storage is unavailable. */
        }
      }
      lastAttempt.current = Date.now();
      inFlight.current = true;
      setLoading(true);
      setError(undefined);
      try {
        const result = await loadReleaseHighlights(revision);
        if (!alive.current) return;
        if (!unreadReleaseHighlights(result, acknowledged.current).length) save(result.sequence);
        setFeed(result);
      } catch (reason) {
        if (alive.current)
          setError(reason instanceof Error ? reason.message : "Release highlights aren’t available yet.");
      } finally {
        inFlight.current = false;
        if (alive.current) setLoading(false);
      }
    },
    [revision, save, feed],
  );

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (!ready || feed) return;
    const refresh = () => {
      if (!document.hidden) void load();
    };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [ready, load, feed]);
  useEffect(() => {
    const synchronize = (event: StorageEvent) => {
      if (event.key !== storageKey) return;
      const sequence = Number(event.newValue);
      if (Number.isSafeInteger(sequence) && sequence > 0) {
        acknowledged.current = Math.max(sequence, acknowledged.current ?? 0);
        if (feed && sequence >= feed.sequence) setAutomaticOpen(false);
      }
    };
    window.addEventListener("storage", synchronize);
    return () => window.removeEventListener("storage", synchronize);
  }, [feed]);
  useEffect(() => {
    if (manualOpen) {
      setShowHistory(true);
      void load(true);
    }
  }, [manualOpen, load]);
  useEffect(() => {
    if (!ready || blocked || open || !feed || !unreadReleaseHighlights(feed, acknowledged.current).length) return;
    const present = () => {
      if (
        !document.hidden &&
        unreadReleaseHighlights(feed, acknowledged.current).length &&
        !document.querySelector("dialog[open]")
      ) {
        setShowHistory(false);
        setAutomaticOpen(true);
      }
    };
    present();
    const timer = window.setInterval(present, 1000);
    return () => clearInterval(timer);
  }, [ready, blocked, open, feed]);
  useEffect(() => {
    if (!open) return;
    const element = dialog.current!;
    const opener = document.activeElement as HTMLElement | null;
    element.showModal();
    element.querySelector<HTMLElement>("[data-initial-focus]")?.focus();
    return () => {
      element.close();
      if (opener?.isConnected && opener !== document.body) opener.focus();
      else {
        const settings = document.querySelector<HTMLElement>('button[aria-label="User Settings"]');
        (settings?.getClientRects().length
          ? settings
          : document.querySelector<HTMLElement>('button[aria-label="Back to Browse"]')
        )?.focus();
      }
    };
  }, [open]);

  const close = () => {
    if (feed) save(feed.sequence);
    setAutomaticOpen(false);
    onManualClose();
  };
  const releases = feed
    ? showHistory
      ? feed.releases.filter((release) => release.highlights.length).reverse()
      : unreadReleaseHighlights(feed, acknowledged.current)
    : [];
  return open ? (
    <dialog
      ref={dialog}
      className="audio-dialog whats-new-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      {/* Like the native update review: the face, the title, then the build right under it. */}
      <header className="audio-dialog-heading whats-new-heading">
        <div className="whats-new-title">
          <img className="whats-new-face" src="/caper-face.svg" alt="" width={36} height={36} />
          <h2 id={titleId}>What’s new</h2>
          {sourceRevisionValid(revision) && (
            <a
              className="whats-new-build"
              href={`https://github.com/joswayski/caper/commit/${revision}`}
              target="_blank"
              rel="noreferrer"
              title="View this version on GitHub"
            >
              Web build {revision.slice(0, 7)}
              <ExternalLink size={12} aria-hidden="true" />
            </a>
          )}
        </div>
        <button type="button" className="voice-icon-button" aria-label="Close What’s new" onClick={close}>
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="audio-dialog-body">
        {loading && !feed && <p role="status">Loading release highlights…</p>}
        {!sourceRevisionValid(revision) && <p>Release highlights are available in published builds.</p>}
        {error && (
          <div role="alert">
            <p>{error}</p>
            <button type="button" className="secondary" disabled={loading} onClick={() => void load(true)}>
              Try again
            </button>
          </div>
        )}
        {!loading && !error && feed && !releases.length && (
          <p className="whats-new-empty">No release highlights for this version yet.</p>
        )}
        {releaseHighlightDays(releases).map(({ date, releases: dayReleases }) => (
          <section className="whats-new-release" key={date}>
            <time dateTime={date}>
              {new Date(date).toLocaleDateString(undefined, {
                timeZone: "UTC",
                month: "long",
                day: "numeric",
                year: "numeric",
              })}
            </time>
            {dayReleases.map((release) => (
              <div className="whats-new-entry" key={release.sequence}>
                {release.highlights.map((highlight, index) => (
                  <article key={index}>
                    <h3>{highlight.title}</h3>
                    <a
                      className="whats-new-source"
                      href={`https://github.com/joswayski/caper/commit/${release.revision}`}
                      target="_blank"
                      rel="noreferrer"
                      title="View this version on GitHub"
                    >
                      {release.revision.slice(0, 7)}
                      <ExternalLink size={12} aria-hidden="true" />
                    </a>
                    <p>{highlight.body}</p>
                  </article>
                ))}
              </div>
            ))}
          </section>
        ))}
        {showHistory && !!feed && feed.releases[0].sequence > 1 && (
          <p className="noise-status">Showing the retained release history for this version.</p>
        )}
      </div>
      <footer className="whats-new-footer">
        {!showHistory && (
          <button type="button" className="whats-new-history" onClick={() => setShowHistory(true)}>
            Previous updates
          </button>
        )}
        <button type="button" className="primary" data-initial-focus onClick={close}>
          Got it
        </button>
      </footer>
    </dialog>
  ) : null;
}
