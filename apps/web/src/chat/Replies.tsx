import { EyeOff, File, Mic, Paperclip, X } from "lucide-react";
import { Reply } from "../../../../shared/message-icons.ts";
import { useId, useLayoutEffect, useRef, useState } from "react";
import socketArtwork from "../../../../shared/reply-socket.svg";
import Avatar from "../components/Avatar";
import { gifOf } from "./klipy.ts";
import {
  attachmentsOf,
  isChatAttachment,
  type ChatAttachment,
  type ChatMessage,
  type ChatReplyReference,
} from "./types.ts";
import { attachmentLabel, attachmentsExcerpt, isVoiceAttachment } from "./voice.ts";

export function replyReference(message: ChatMessage): ChatReplyReference {
  const gif = gifOf(message.content);
  const text =
    message.content.text ||
    (message.forward ? "Forwarded message" : gif ? (gif.kind === "sticker" ? "Sticker" : "GIF") : "");
  const characters = Array.from(text);
  return {
    id: message.id,
    author: message.author,
    text: characters.slice(0, 200).join(""),
    ...(characters.length > 200 ? { fullText: text } : {}),
    hasAttachments: !!attachmentsOf(message).length,
    attachments: attachmentsOf(message),
    ...(message.threadRootId ? { threadRootId: message.threadRootId } : {}),
  };
}

function ReplyThumbnail({ file }: { file: ChatAttachment }) {
  const [failed, setFailed] = useState(false);
  const concealed = file.spoiler || file.unavailable || (file.status && file.status !== "ready");
  const source =
    !concealed && (file.kind === "image" || file.kind === "video")
      ? file.previewUrl ||
        (file.kind === "image" && !file.animated && file.contentType !== "image/gif" ? file.url : undefined)
      : undefined;
  const label = file.spoiler
    ? "Spoiler attachment"
    : file.unavailable
      ? "Unavailable attachment"
      : attachmentLabel(file);
  return (
    <span className="chat-reply-thumbnail" title={label}>
      {source && !failed ? (
        <img src={source} alt={label} loading="lazy" onError={() => setFailed(true)} />
      ) : file.spoiler ? (
        <EyeOff size={18} aria-label={label} />
      ) : isVoiceAttachment(file) ? (
        <Mic size={18} aria-label={label} />
      ) : (
        <File size={18} aria-label={label} />
      )}
    </span>
  );
}

export function ReplyContext({
  motionKey,
  reference,
  hidden,
  onOpen,
}: {
  motionKey: string;
  reference: ChatReplyReference;
  hidden: boolean;
  onOpen: () => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const excerpt = useRef<HTMLSpanElement>(null);
  const wire = useRef<SVGPathElement>(null);
  const sourceCup = useRef<SVGGElement>(null);
  const targetCup = useRef<SVGGElement>(null);
  const excerptId = useId();
  const [expanded, setExpanded] = useState(false);
  const [clipped, setClipped] = useState(false);
  const [connection, setConnection] = useState({ sourceX: 57, sourceY: 21, targetX: 0, height: 117 });
  const text =
    (reference.fullText ?? reference.text) ||
    (reference.hasAttachments ? attachmentsExcerpt(reference.attachments) : "Message");
  const files = hidden || reference.deleted ? [] : (reference.attachments ?? []).filter(isChatAttachment);
  const canExpand = !reference.deleted && clipped;
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
        targetX: target ? target.left - bounds.left : 0,
        height: target ? target.top + target.height / 2 - bounds.top : bounds.height + 35,
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
  useLayoutEffect(() => {
    if (hidden) return;
    const element = root.current!;
    const scroller = element.closest<HTMLElement>(".chat-scroller, .chat-thread-messages, .chat-pins");
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    // Seed from the replying message, not its quoted source: even replies to the
    // same original have their own weight, tension and settling time on remount.
    let seed = 2166136261;
    for (const byte of new TextEncoder().encode(motionKey)) seed = Math.imul(seed ^ byte, 16777619) >>> 0;
    const stiffness = 560 + ((seed & 255) / 255) * 400;
    const damping = 28 + (((seed >>> 8) & 255) / 255) * 14;
    const impulse = 1 + (((seed >>> 16) & 255) / 255) * 0.4;
    const limit = 5 + ((seed >>> 24) / 255) * 2;
    const speedLimit = limit * 16;
    let frame = 0;
    let previousTime = 0;
    let previousScroll = scroller?.scrollTop ?? 0;
    let offset = 0;
    let velocity = 0;
    const draw = () => {
      // Both cup mouths stay on their avatars. The string follows the rotated backs.
      const angle = offset * 0.025;
      const dx = 12 * Math.cos(angle);
      const dy = 12 * Math.sin(angle);
      const startX = targetX - dx;
      const startY = height + dy;
      const endX = sourceX - dx;
      const endY = sourceY - dy;
      const span = startY - endY;
      // Bend inward from the lower cup, even at maximum sway, away from the row edge.
      wire.current!.setAttribute(
        "d",
        `M${startX} ${startY}C${startX + 8 + offset} ${startY - span * 0.3} ${endX - 30 + offset} ${endY + span * 0.3} ${endX} ${endY}`,
      );
      sourceCup.current!.setAttribute(
        "transform",
        `translate(${sourceX} ${sourceY}) rotate(${(angle * 180) / Math.PI})`,
      );
      targetCup.current!.setAttribute(
        "transform",
        `translate(${targetX} ${height}) rotate(${(-angle * 180) / Math.PI})`,
      );
    };
    const stop = () => {
      cancelAnimationFrame(frame);
      frame = 0;
      offset = velocity = 0;
      draw();
    };
    const step = (time: number) => {
      const dt = Math.min((time - previousTime) / 1000, 0.032);
      previousTime = time;
      velocity -= (stiffness * offset + damping * velocity) * dt;
      offset = Math.max(-limit, Math.min(limit, offset + velocity * dt));
      if (Math.abs(offset) < 0.01 && Math.abs(velocity) < 0.05) stop();
      else {
        draw();
        frame = requestAnimationFrame(step);
      }
    };
    const scroll = () => {
      const delta = scroller!.scrollTop - previousScroll;
      previousScroll = scroller!.scrollTop;
      const bounds = element.getBoundingClientRect();
      const viewport = scroller!.getBoundingClientRect();
      if (motion.matches || bounds.bottom + 35 < viewport.top || bounds.top > viewport.bottom) {
        stop();
        return;
      }
      velocity = Math.max(-speedLimit, Math.min(speedLimit, velocity + delta * impulse));
      if (!frame) {
        previousTime = performance.now();
        frame = requestAnimationFrame(step);
      }
    };
    draw();
    scroller?.addEventListener("scroll", scroll, { passive: true });
    motion.addEventListener("change", stop);
    return () => {
      cancelAnimationFrame(frame);
      scroller?.removeEventListener("scroll", scroll);
      motion.removeEventListener("change", stop);
    };
  }, [connection, hidden, motionKey]);
  return (
    <div
      ref={root}
      className="chat-reply-context"
      data-expanded={expanded && !hidden}
      data-clipped={canExpand && !hidden}
    >
      {!hidden && (
        <svg
          className="chat-reply-connector"
          style={{ width: sourceX + 1, height }}
          viewBox={`0 0 ${sourceX + 1} ${height}`}
          aria-hidden="true"
        >
          <path ref={wire} className="chat-reply-line chat-reply-route" />
          <g ref={sourceCup} className="chat-reply-socket chat-reply-socket-source">
            <image href={socketArtwork} x="-14" y="-6" width="16" height="12" />
          </g>
          <g ref={targetCup} className="chat-reply-socket chat-reply-socket-target">
            <image href={socketArtwork} x="-14" y="-6" width="16" height="12" />
          </g>
        </svg>
      )}
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
                  <Avatar person={reference.author} name={reference.author.name} />
                </span>
                <strong>{reference.author.name}</strong>
                {reference.hasAttachments && <Paperclip size={13} aria-label="Has attachments" />}
              </>
            )}
            <span className="chat-reply-view">View original</span>
          </span>
          {!hidden && (
            <span
              ref={excerpt}
              id={excerptId}
              className={`chat-reply-excerpt${reference.deleted ? " chat-reply-excerpt-deleted" : ""}`}
            >
              {reference.deleted ? "Original message was deleted." : text}
            </span>
          )}
          {!!files.length && (
            <span className="chat-reply-attachments" aria-label="Original message attachments">
              {files.slice(0, 3).map((file) => (
                <ReplyThumbnail key={file.id} file={file} />
              ))}
              {files.length > 3 && <span className="chat-reply-thumbnail">+{files.length - 3}</span>}
            </span>
          )}
        </button>
        {!hidden && canExpand && (
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
      <Reply size={14} aria-hidden="true" />
      <div className="chat-reply-draft-content">
        <span className="chat-reply-draft-heading">
          Replying to <strong>{hidden ? "blocked message" : reference.author.name}</strong>
        </span>
        {!hidden && (
          <span className="chat-reply-draft-excerpt">
            {(reference.fullText ?? reference.text) ||
              (reference.hasAttachments ? attachmentsExcerpt(reference.attachments) : "Message")}
          </span>
        )}
      </div>
      <button type="button" aria-label="Cancel reply" onClick={onCancel}>
        <X size={16} />
      </button>
    </div>
  );
}
