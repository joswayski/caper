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
  const [connection, setConnection] = useState({ sourceX: 57, sourceY: 21, targetX: 17, height: 100 });
  const text = reference.fullText ?? reference.text;
  useLayoutEffect(() => {
    const element = root.current!;
    const update = () => {
      const bounds = element.getBoundingClientRect();
      const avatar = element.parentElement!.querySelector<HTMLElement>(":scope > .chat-avatar");
      const target = avatar?.getBoundingClientRect();
      const source = element
        .querySelector<HTMLElement>(".chat-reply-avatar, .chat-reply-heading")!
        .getBoundingClientRect();
      setConnection({
        sourceX: source.left - bounds.left,
        sourceY: source.top + source.height / 2 - bounds.top,
        targetX: target ? target.left + target.width / 2 - bounds.left : 17,
        height: target ? target.top - bounds.top + 1 : bounds.height + 18,
      });
      // The same context appears in the timeline, threads, pending rows and Pins.
      const actions =
        element.parentElement!.querySelector<HTMLElement>(":scope > .chat-pin-marker")?.getBoundingClientRect() ??
        target;
      element.parentElement!.style.setProperty(
        "--reply-actions-top",
        `${(actions?.top ?? bounds.bottom + 18) - element.parentElement!.getBoundingClientRect().top - 6}px`,
      );
      const content = excerpt.current;
      setClipped(!!content && content.scrollHeight > parseFloat(getComputedStyle(content).lineHeight) * 3 + 1);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    observer.observe(element.parentElement!);
    return () => {
      observer.disconnect();
      element.parentElement?.style.removeProperty("--reply-actions-top");
    };
  }, [hidden, text]);
  const { sourceX, sourceY, targetX, height } = connection;
  const bendX = sourceX - 12;
  const bendY = sourceY + 28;
  const path = `M${targetX} ${height}V${bendY}Q${targetX} ${sourceY} ${bendX} ${sourceY}H${sourceX}`;
  const ribbon = `M${targetX - 0.55} ${height}L${targetX - 1.6} ${bendY}Q${targetX - 1.6} ${sourceY - 2} ${bendX} ${sourceY - 2}H${sourceX}V${sourceY + 2}H${bendX}Q${targetX + 1.6} ${sourceY + 2} ${targetX + 1.6} ${bendY}L${targetX + 0.55} ${height}Z`;
  const clipId = `reply-ribbon-${excerptId}`;
  return (
    <div ref={root} className="chat-reply-context" data-expanded={expanded && !hidden}>
      <svg
        className="chat-reply-connector"
        style={{ width: sourceX + 1, height }}
        viewBox={`0 0 ${sourceX + 1} ${height}`}
        aria-hidden="true"
      >
        <defs>
          <clipPath id={clipId}>
            <path d={ribbon} />
          </clipPath>
        </defs>
        <path className="chat-reply-line" d={ribbon} />
        {!hidden && <path className="chat-reply-flow chat-reply-glow" d={path} pathLength="100" />}
        {!hidden && <path className="chat-reply-flow" d={path} pathLength="100" clipPath={`url(#${clipId})`} />}
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
