import { useEffect, useRef, useState } from "react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingOverlay,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { X } from "lucide-react";
import Avatar from "../components/Avatar";
import type { ChatMessage } from "./types.ts";
import { emojiAsset, emojiCode } from "./emoji.ts";
import { cachedReactors, emojiLabel, emojiName, loadReactors, reactorName, type ReactionList } from "./reactors.ts";

export interface ReactorsTarget {
  messageId: string;
  emoji: string;
  anchor: HTMLElement;
  drawer: boolean;
}

/** Who reacted, by emoji: a bottom sheet on touch, a popover with a pointer. */
export default function ReactorsPanel({
  channelId,
  message,
  target,
  onClose,
}: {
  channelId: string;
  message: ChatMessage;
  target: ReactorsTarget;
  onClose: () => void;
}) {
  const reactions = message.reactions ?? [];
  const [selected, setSelected] = useState(target.emoji);
  const [list, setList] = useState<ReactionList | undefined>(() =>
    cachedReactors(channelId, message.id, message.reactionSeq),
  );
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const [name, setName] = useState<string>();
  const returnFocus = useRef(target.anchor);
  const { refs, floatingStyles, context } = useFloating({
    open: true,
    onOpenChange: (open) => {
      if (!open) onClose();
    },
    placement: "top-start",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context)]);
  useEffect(() => {
    refs.setReference(target.anchor);
  }, [target.anchor, refs.setReference]);

  // Follow the live snapshot: a removed emoji moves the selection; none closes.
  const current = reactions.some((reaction) => reaction.emoji === selected) ? selected : reactions[0]?.emoji;
  useEffect(() => {
    if (!current) onClose();
    else if (current !== selected) setSelected(current);
  }, [current, selected, onClose]);

  useEffect(() => {
    let active = true;
    setError(undefined);
    loadReactors(channelId, message.id, message.reactionSeq).then(
      (loaded) => {
        if (active) setList(loaded);
      },
      (reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : "Reactions are unavailable.");
      },
    );
    return () => {
      active = false;
    };
  }, [channelId, message.id, message.reactionSeq, attempt]);

  useEffect(() => {
    let active = true;
    setName(undefined);
    if (current)
      void emojiName(current).then((found) => {
        if (active) setName(found);
      });
    return () => {
      active = false;
    };
  }, [current]);

  if (!current) return null;
  const authors = list?.reactions.find((reaction) => reaction.emoji === current)?.authors;
  const panel = (
    <FloatingFocusManager context={context} returnFocus={returnFocus}>
      <div
        className={`chat-reactors${target.drawer ? " chat-reactors-drawer" : ""}`}
        ref={refs.setFloating}
        style={target.drawer ? undefined : floatingStyles}
        aria-label="Reactions"
        {...getFloatingProps()}
      >
        {target.drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-reaction-picker-heading">
          <strong>Reactions</strong>
          <button type="button" aria-label="Close reactions" onClick={onClose}>
            <X size={18} />
          </button>
        </div>
        <div className="chat-reactors-tabs" role="tablist" aria-label="Reactions by emoji">
          {reactions.map((reaction) => (
            <button
              type="button"
              role="tab"
              key={reaction.emoji}
              aria-selected={reaction.emoji === current}
              aria-label={`${reaction.emoji}, ${reaction.authorIds.length}`}
              onClick={() => setSelected(reaction.emoji)}
            >
              <img src={emojiAsset(emojiCode(reaction.emoji))} width={20} height={20} alt="" />
              <span>{reaction.authorIds.length}</span>
            </button>
          ))}
        </div>
        <div className="chat-reactors-label">{emojiLabel(current, name)}</div>
        {authors ? (
          <ul
            className="chat-reactors-list"
            role="tabpanel"
            aria-label={`People who reacted with ${emojiLabel(current, name)}`}
          >
            {authors.map((author) => (
              <li key={author.id}>
                <span className="chat-avatar chat-reactors-avatar">
                  <Avatar avatarId={author.avatarId} name={reactorName(author)} />
                </span>
                <span>
                  <strong>{reactorName(author)}</strong>
                  {author.username && <span className="chat-reactors-username">@{author.username}</span>}
                </span>
              </li>
            ))}
          </ul>
        ) : error ? (
          <div className="chat-reactors-status" role="alert">
            <span>Couldn’t load reactions.</span>
            <button type="button" onClick={() => setAttempt((count) => count + 1)}>
              Retry
            </button>
          </div>
        ) : (
          <p className="chat-reactors-status" role="status">
            Loading reactions…
          </p>
        )}
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingPortal root={target.anchor.closest<HTMLElement>(".live-scene") ?? undefined}>
      {target.drawer ? (
        <FloatingOverlay lockScroll className="chat-actions-overlay">
          {panel}
        </FloatingOverlay>
      ) : (
        panel
      )}
    </FloatingPortal>
  );
}
