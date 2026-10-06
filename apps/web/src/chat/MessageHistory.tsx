import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { loadMessageVersions } from "./client.ts";
import { textChange } from "./edits.ts";
import type { ChatMessage, MessageVersion } from "./types.ts";

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
    setLoading(true); setError(undefined);
    try {
      const page = await loadMessageVersions(message.channelId, message.id, older ? versions.at(-1)?.revision : undefined, signal);
      if (signal.aborted) return;
      setVersions((current) => older ? [...current, ...page.versions.filter((version) => !current.some((item) => item.revision === version.revision))] : page.versions);
      setHasMore(page.hasMore);
      if (!older) setSelected(page.versions[0]?.revision);
    } catch (failure) {
      if (!signal.aborted) setError(failure instanceof Error ? failure.message : "Couldn’t load message history.");
    } finally { if (!signal.aborted) setLoading(false); }
  };
  useEffect(() => {
    // The dialog's key follows the content revision, so live edits refresh history.
    controller.current = new AbortController();
    dialog.current?.showModal(); void load();
    return () => controller.current.abort();
  }, []);
  const version = versions.find((item) => item.revision === selected);
  const previous = versions.find((item) => item.revision === (selected ?? 0) - 1);
  const diff = version && previous ? textChange(previous.content.text, version.content.text) : undefined;
  return <dialog ref={dialog} className="chat-edit-dialog chat-version-dialog" aria-labelledby="chat-version-heading"
    onCancel={(event) => { event.preventDefault(); onClose(); }}>
    <header><h2 id="chat-version-heading">Message history</h2><button type="button" aria-label="Close message history" onClick={onClose}><X size={18} /></button></header>
    <p className="chat-edit-notice">{message.author.name} · Previous versions are retained.</p>
    {versions.length > 0 && <>
      <label htmlFor="chat-version-select">Version</label>
      <select id="chat-version-select" value={selected} onChange={(event) => setSelected(Number(event.target.value))}>
        {versions.map((item) => <option value={item.revision} key={item.revision}>{item.revision === 1 ? "Original" : `Version ${item.revision}`} · {new Date(item.createdAt).toLocaleString()}</option>)}
      </select>
      {version && <><h3>{version.revision === 1 ? "Original message" : `Version ${version.revision}`}</h3><p className="chat-version-text">{version.content.text}</p></>}
      {diff && <><h3>Changes from version {previous!.revision}</h3><p className="chat-diff-legend"><span>− Removed</span><span>+ Added</span></p>
        <p className="chat-version-text chat-version-diff">{diff.prefix}{diff.removed && <del>{diff.removed}</del>}{diff.added && <ins>{diff.added}</ins>}{diff.suffix}</p>
      </>}
      {version && version.revision > 1 && !previous && hasMore && <p className="chat-edit-notice">Load older versions to compare this change.</p>}
    </>}
    {loading && <p role="status">Loading versions…</p>}
    {error && <p role="alert" className="chat-action-error">{error} <button type="button" onClick={() => void load(versions.length > 0)}>Retry</button></p>}
    {hasMore && !error && <button type="button" disabled={loading} onClick={() => void load(true)}>Load older versions</button>}
  </dialog>;
}
