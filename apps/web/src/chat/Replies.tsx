import { CornerUpRight, Paperclip, X } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import Avatar from "../components/Avatar";
import { attachmentsOf, type ChatMessage, type ChatReplyReference } from "./types.ts";

export function replyReference(message: ChatMessage): ChatReplyReference {
  const text = message.content.text || (message.forward ? "Forwarded message" : "");
  const characters = Array.from(text);
  return {
    id: message.id,
    author: message.author,
    text: characters.slice(0, 200).join(""),
    ...(characters.length > 200 ? { fullText: text } : {}),
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
  const root = useRef<HTMLDivElement>(null);
  const excerpt = useRef<HTMLSpanElement>(null);
  const excerptId = useId();
  const [expanded, setExpanded] = useState(false);
  const [clipped, setClipped] = useState(false);
  const [height, setHeight] = useState(44);
  const text = reference.fullText ?? reference.text;
  useLayoutEffect(() => {
    const element = root.current!;
    const update = () => {
      const bounds = element.getBoundingClientRect();
      const avatar = element.parentElement!.querySelector<HTMLElement>(":scope > .chat-avatar");
      setHeight(avatar ? avatar.getBoundingClientRect().top - bounds.top + 1 : bounds.height + 6);
      // The same context appears in the timeline, threads, pending rows and Pins.
      element.parentElement!.style.setProperty("--reply-context-height", `${bounds.height}px`);
      const content = excerpt.current;
      setClipped(!!content && content.scrollHeight > parseFloat(getComputedStyle(content).lineHeight) * 3 + 1);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    observer.observe(element.parentElement!);
    return () => {
      observer.disconnect();
      element.parentElement?.style.removeProperty("--reply-context-height");
    };
  }, [hidden, text]);
  const path = `M44 10H24Q17 10 17 17V${height}`;
  return (
    <div ref={root} className="chat-reply-context" data-expanded={expanded && !hidden}>
      <svg className="chat-reply-connector" style={{ height }} viewBox={`0 0 44 ${height}`} aria-hidden="true">
        <path d={path} />
        <path className="chat-reply-flow" d={path} pathLength="100" />
      </svg>
      <div className="chat-reply-body">
        <button
          type="button"
          className="chat-reply-source"
          onClick={onOpen}
          aria-label={hidden ? "View original blocked message" : `View original message from ${reference.author.name}`}
          aria-describedby={hidden ? undefined : excerptId}
        >
          <span className="chat-reply-heading">
            {hidden ? (
              <span>Blocked message</span>
            ) : (
              <>
                <span className="chat-reply-avatar">
                  <Avatar avatarId={reference.author.avatarId} name={reference.author.name} />
                </span>
                <strong>{reference.author.name}</strong>
                {reference.hasAttachments && <Paperclip size={13} aria-label="Has attachments" />}
              </>
            )}
            <span className="chat-reply-view">View original</span>
          </span>
          {!hidden && (
            <span ref={excerpt} id={excerptId} className="chat-reply-excerpt">
              {text || (reference.hasAttachments ? "Attachment" : "Message")}
            </span>
          )}
        </button>
        {!hidden && clipped && (
          <button
            type="button"
            className="chat-reply-expand"
            aria-expanded={expanded}
            aria-controls={excerptId}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "Show less" : "Show more"}
          </button>
        )}
      </div>
    </div>
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
      <CornerUpRight size={14} aria-hidden="true" />
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
