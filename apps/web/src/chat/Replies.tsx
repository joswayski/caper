import { CornerUpLeft, Paperclip, X } from "lucide-react";
import Avatar from "../components/Avatar";
import { attachmentsOf, type ChatMessage, type ChatReplyReference } from "./types.ts";

export function replyReference(message: ChatMessage): ChatReplyReference {
  return {
    id: message.id,
    author: message.author,
    text: Array.from(message.content.text || (message.forward ? "Forwarded message" : ""))
      .slice(0, 200)
      .join(""),
    hasAttachments: !!attachmentsOf(message).length,
    ...(message.threadRootId ? { threadRootId: message.threadRootId } : {}),
  };
}

export function ReplyContext({
  reference,
  hidden,
  onOpen,
}: {
  reference: ChatReplyReference;
  hidden: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      className="chat-reply-context"
      onClick={onOpen}
      aria-label={hidden ? "Go to blocked message" : `Go to message from ${reference.author.name}`}
    >
      <CornerUpLeft size={14} aria-hidden="true" />
      {hidden ? (
        <span>Blocked message</span>
      ) : (
        <>
          <span className="chat-reply-avatar">
            <Avatar avatarId={reference.author.avatarId} name={reference.author.name} />
          </span>
          <strong>{reference.author.name}</strong>
          <span>{reference.text || (reference.hasAttachments ? "Attachment" : "Message")}</span>
          {reference.hasAttachments && <Paperclip size={13} aria-label="Has attachments" />}
        </>
      )}
    </button>
  );
}

export function ReplyDraft({
  reference,
  hidden,
  onCancel,
}: {
  reference: ChatReplyReference;
  hidden: boolean;
  onCancel: () => void;
}) {
  return (
    <div className="chat-reply-draft" role="status">
      <CornerUpLeft size={14} aria-hidden="true" />
      <span>
        Replying to <strong>{hidden ? "blocked message" : reference.author.name}</strong>
      </span>
      {!hidden && (
        <span className="chat-reply-draft-excerpt">
          {reference.text || (reference.hasAttachments ? "Attachment" : "Message")}
        </span>
      )}
      <button type="button" aria-label="Cancel reply" onClick={onCancel}>
        <X size={16} />
      </button>
    </div>
  );
}
