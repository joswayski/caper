import { useEffect, useRef, useState } from "react";
import { dialogKeyDown } from "../components/dialogKeyboard";
import { focusReturn } from "./focus-return.ts";
import type { ChatMessage } from "./types.ts";

export default function DeleteMessage({
  message,
  onDelete,
  onClose,
}: {
  message: ChatMessage;
  onDelete: () => Promise<void>;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    const restore = focusReturn(message.clientMessageId);
    dialog.current?.showModal();
    return restore;
  }, []);
  const remove = async () => {
    if (deleting) return;
    setDeleting(true);
    setError(undefined);
    try {
      await onDelete();
      onClose();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : "Couldn’t delete the message. Try again.");
      setDeleting(false);
    }
  };
  return (
    <dialog
      ref={dialog}
      className="chat-edit-dialog chat-delete-dialog"
      aria-labelledby="chat-delete-heading"
      aria-describedby="chat-delete-notice"
      onKeyDown={dialogKeyDown}
      onCancel={(event) => {
        event.preventDefault();
        if (!deleting) onClose();
      }}
    >
      <h2 id="chat-delete-heading">Delete message?</h2>
      <p id="chat-delete-notice" className="chat-edit-notice">
        This removes the message for everyone. This can’t be undone.
      </p>
      {error && (
        <p role="alert" className="chat-action-error">
          {error}
        </p>
      )}
      <div className="chat-edit-buttons">
        <button type="button" autoFocus disabled={deleting} onClick={onClose}>
          Cancel
        </button>
        <button type="button" className="chat-delete-confirm" disabled={deleting} onClick={() => void remove()}>
          {deleting ? "Deleting…" : "Delete"}
        </button>
      </div>
    </dialog>
  );
}
