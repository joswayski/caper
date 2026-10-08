import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { loadMessageVersions } from "./client.ts";
import type { ChatMessage, MessageVersion } from "./types.ts";

// History opens after a browser interaction; don't bundle the DOM renderer into the server.
const MessageDiff = lazy<typeof import("./MessageDiff.tsx").default>(() =>
  import.meta.env.SSR
    ? Promise.resolve({ default: () => <></> })
    : import("./MessageDiff.tsx").then(async (module) => {
        await module.ready;
        return module;
      }),
);
const at = (version: MessageVersion) => `Version ${version.revision} · ${new Date(version.createdAt).toLocaleString()}`;

function Comparison({
  messageId,
  before,
  after,
  current = false,
}: {
  messageId: string;
  before: MessageVersion;
  after: MessageVersion;
  current?: boolean;
}) {
  return (
    <section
      className="chat-version-comparison"
      aria-label={current ? "Latest change" : `Change to version ${after.revision}`}
    >
      <div className="chat-version-columns">
        <div>
          <h3>{current ? "Previous version" : "Before"}</h3>
          <p>{at(before)}</p>
        </div>
        <div>
          <h3>{current ? "Current version" : "After"}</h3>
          <p>{at(after)}</p>
        </div>
      </div>
      <Suspense
        fallback={
          <div className="chat-version-columns">
            <p className="chat-version-text">{before.content.text}</p>
            <p className="chat-version-text">{after.content.text}</p>
          </div>
        }
      >
        <MessageDiff messageId={messageId} before={before} after={after} />
      </Suspense>
    </section>
  );
}

export default function MessageHistory({ message, onClose }: { message: ChatMessage; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const controller = useRef(new AbortController());
  const [versions, setVersions] = useState<MessageVersion[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [selected, setSelected] = useState<number>();
  const load = async (older = false) => {
    const signal = controller.current.signal;
    setLoading(true);
    setError(undefined);
    try {
      const page = await loadMessageVersions(
        message.channelId,
        message.id,
        older ? versions.at(-1)?.revision : undefined,
        signal,
      );
      if (signal.aborted) return;
      setVersions((current) =>
        older
          ? [
              ...current,
              ...page.versions.filter((version) => !current.some((item) => item.revision === version.revision)),
            ]
          : page.versions,
      );
      setHasMore(page.hasMore);
      if (!older) setSelected(undefined);
    } catch (failure) {
      if (!signal.aborted) setError(failure instanceof Error ? failure.message : "Couldn’t load message history.");
    } finally {
      if (!signal.aborted) setLoading(false);
    }
  };
  useEffect(() => {
    // The dialog's key follows the content revision, so live edits refresh history.
    controller.current = new AbortController();
    dialog.current?.showModal();
    void load();
    return () => controller.current.abort();
  }, []);
  const current = versions[0],
    latestPrevious = versions[1];
  const version = versions.find((item) => item.revision === selected);
  const previous = versions.find((item) => item.revision === (selected ?? 0) - 1);
  return (
    <dialog
      ref={dialog}
      className="chat-edit-dialog chat-version-dialog"
      aria-labelledby="chat-version-heading"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <header>
        <h2 id="chat-version-heading">Message history</h2>
        <button type="button" aria-label="Close message history" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      <p className="chat-edit-notice">{message.author.name} · Previous versions are retained.</p>
      {current && latestPrevious && (
        <Comparison messageId={message.id} before={latestPrevious} after={current} current />
      )}
      {current && !latestPrevious && (
        <>
          <h3>Original version</h3>
          <p className="chat-version-text">{current.content.text}</p>
        </>
      )}
      {versions.length > 1 && (
        <div className="chat-version-older">
          <label htmlFor="chat-version-select">View previous versions</label>
          <select
            id="chat-version-select"
            value={selected ?? ""}
            onChange={(event) => setSelected(event.target.value ? Number(event.target.value) : undefined)}
          >
            <option value="">Choose an earlier version…</option>
            {versions.slice(1).map((item) => (
              <option value={item.revision} key={item.revision}>
                {item.revision === 1 ? "Original version" : `Version ${item.revision}`} ·{" "}
                {new Date(item.createdAt).toLocaleString()}
              </option>
            ))}
          </select>
          {version && previous && <Comparison messageId={message.id} before={previous} after={version} />}
          {version && !previous && (
            <>
              <h3>{version.revision === 1 ? "Original version" : `Version ${version.revision}`}</h3>
              <p className="chat-version-text chat-version-original">{version.content.text}</p>
            </>
          )}
          {version && version.revision > 1 && !previous && hasMore && (
            <p className="chat-edit-notice">Load older versions to compare this change.</p>
          )}
        </div>
      )}
      {loading && <p role="status">Loading versions…</p>}
      {error && (
        <p role="alert" className="chat-action-error">
          {error}{" "}
          <button type="button" disabled={loading} onClick={() => void load(versions.length > 0)}>
            Retry
          </button>
        </p>
      )}
      {hasMore && !error && (
        <button type="button" disabled={loading} onClick={() => void load(true)}>
          Load older versions
        </button>
      )}
    </dialog>
  );
}
