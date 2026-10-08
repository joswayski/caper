import { useEffect, useRef, useState } from "react";
import type { ChatMessage } from "./types.ts";

export default function MessageEditor({
  message,
  onSave,
  onReload,
  onClose,
}: {
  message: ChatMessage;
  onSave: (text: string, expectedRevision: number) => Promise<void>;
  onReload: () => Promise<ChatMessage>;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const [baseline, setBaseline] = useState(message);
  const [draft, setDraft] = useState(message.content.text);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const count = Array.from(draft).length;
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  const save = async () => {
    if (saving || !draft.trim() || count > 4_000) return;
    setSaving(true);
    setError(undefined);
    try {
      await onSave(draft, baseline.revision ?? 1);
      onClose();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : "Edit could not be saved. Your draft is kept.");
    } finally {
      setSaving(false);
      requestAnimationFrame(() => textarea.current?.focus());
    }
  };
  const reload = async () => {
    setSaving(true);
    setError(undefined);
    try {
      const latest = await onReload();
      setBaseline(latest);
      setDraft(latest.content.text);
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : "Couldn’t load the latest version.");
    } finally {
      setSaving(false);
      requestAnimationFrame(() => textarea.current?.focus());
    }
  };
  return (
    <dialog
      ref={dialog}
      className="chat-edit-dialog"
      aria-labelledby="chat-edit-heading"
      onCancel={(event) => {
        event.preventDefault();
        if (!saving) onClose();
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <h2 id="chat-edit-heading">Edit message</h2>
        <p className="chat-edit-notice">Previous versions remain visible to people who can read this message.</p>
        <label htmlFor="chat-edit-text">Message</label>
        <textarea
          ref={textarea}
          id="chat-edit-text"
          autoFocus
          value={draft}
          disabled={saving}
          rows={5}
          aria-describedby="chat-edit-count"
          aria-invalid={count > 4_000 || undefined}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void save();
            }
          }}
        />
        <small id="chat-edit-count" className={count > 4_000 ? "chat-action-error" : ""}>
          {count.toLocaleString()} / 4,000
        </small>
        {error && (
          <div role="alert" className="chat-action-error">
            <p>{error}</p>
            <button type="button" disabled={saving} onClick={() => void reload()}>
              Discard draft and load latest
            </button>
          </div>
        )}
        <div className="chat-edit-buttons">
          <button type="button" disabled={saving} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="chat-edit-save" disabled={saving || !draft.trim() || count > 4_000}>
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </form>
    </dialog>
  );
}
