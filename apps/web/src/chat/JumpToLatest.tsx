import { useEffect, useRef, useState } from "react";
import { ArrowDown } from "lucide-react";

interface TimelineEntry {
  clientMessageId: string;
  author?: { id: string };
}

const gap = (scroller: HTMLElement) => scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
/** Keeps the message the reader left at while they stay away; forgets it at the live end. */
const position =
  (scroller: HTMLElement, latest: string | undefined) => (current: { key: string | undefined } | undefined) =>
    gap(scroller) > scroller.clientHeight / 2 ? (current ?? { key: latest }) : undefined;

/**
 * A floating way back to the newest message, shown once the reader is more than
 * half a screen above it. Counts what other people post meanwhile. It keeps its
 * own scroll state, so scrolling never re-renders the conversation around it.
 */
export default function JumpToLatest({
  scroller,
  messages,
  viewerId,
  blockedIds,
  onJump,
}: {
  scroller: HTMLElement | null;
  /** The loaded messages, oldest first, reaching the live end. */
  messages: readonly TimelineEntry[];
  viewerId?: string;
  blockedIds?: ReadonlySet<string>;
  /** Scroll to the newest message and follow new ones again. */
  onJump: (smooth: boolean) => void;
}) {
  const latest = messages.at(-1)?.clientMessageId;
  const latestRef = useRef(latest);
  latestRef.current = latest;
  // The newest message when the reader left the live end; later ones are new.
  const [leftAt, setLeftAt] = useState<{ key: string | undefined }>();
  useEffect(() => {
    if (!scroller) return setLeftAt(undefined);
    const update = () => setLeftAt(position(scroller, latestRef.current));
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    // Rows that grow or a window that shrinks move the live end without a scroll event.
    const observer = new ResizeObserver(update);
    observer.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", update);
      observer.disconnect();
    };
  }, [scroller]);
  // Messages arriving under a reader who is not scrolling move the live end
  // without a scroll event: look again once the new rows are laid out.
  useEffect(() => {
    if (!scroller) return;
    const frame = requestAnimationFrame(() => setLeftAt(position(scroller, latestRef.current)));
    return () => cancelAnimationFrame(frame);
  }, [scroller, latest, messages.length]);
  if (!leftAt || !scroller) return null;

  const since = messages.findIndex((message) => message.clientMessageId === leftAt.key);
  const added = since === -1 ? 0 : messages.length - since - 1;
  let count = 0;
  for (const message of added ? messages.slice(-added) : []) {
    const author = message.author?.id;
    if (author && author !== viewerId && !blockedIds?.has(author)) count++;
  }
  const label = count ? `${count > 99 ? "99+" : count} new ${count === 1 ? "message" : "messages"}` : undefined;
  return (
    <button
      type="button"
      className="chat-jump-latest"
      data-new={count > 0 || undefined}
      aria-label={label ? `Jump to latest, ${label}` : "Jump to latest"}
      title="Jump to latest"
      onClick={() =>
        onJump(
          gap(scroller) <= scroller.clientHeight * 3 && !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
        )
      }
    >
      <ArrowDown size={16} aria-hidden="true" />
      {label}
    </button>
  );
}
