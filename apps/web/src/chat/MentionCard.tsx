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
import Avatar from "../components/Avatar";
import type { MentionCardPerson } from "./mentions.ts";

export interface MentionCardTarget {
  person: MentionCardPerson;
  anchor: HTMLElement;
  /** Phones and touch layouts get the bottom sheet used by message actions. */
  drawer: boolean;
}

/** Who a mention points at, with Message to open or start a DM. */
export default function MentionCard({
  target,
  onClose,
  onMessage,
}: {
  target: MentionCardTarget;
  onClose: () => void;
  onMessage?: (username: string) => Promise<void>;
}) {
  const { person, anchor, drawer } = target;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const returnFocus = useRef(anchor);
  const { refs, floatingStyles, context } = useFloating({
    open: true,
    onOpenChange: (open) => {
      if (!open) onClose();
    },
    placement: "bottom-start",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context)]);
  useEffect(() => {
    refs.setReference(anchor);
  }, [anchor, refs.setReference]);
  const title = person.displayName ?? `@${person.username}`;

  const message = async () => {
    if (!onMessage || pending) return;
    setPending(true);
    setError(undefined);
    try {
      await onMessage(person.username);
      onClose();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Couldn’t open a conversation.");
      setPending(false);
    }
  };

  const panel = (
    <FloatingFocusManager context={context} returnFocus={returnFocus}>
      <div
        className={`chat-mention-card${drawer ? " chat-mention-card-drawer" : ""}`}
        ref={refs.setFloating}
        style={drawer ? undefined : floatingStyles}
        aria-label={`Profile for ${title}`}
        {...getFloatingProps()}
      >
        {drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-mention-card-identity">
          <i className="chat-mention-card-avatar">
            <Avatar avatarId={person.avatarId} name={person.displayName ?? person.username} />
          </i>
          <div>
            <strong>{title}</strong>
            {person.displayName && <span>@{person.username}</span>}
          </div>
        </div>
        {person.self ? (
          <p className="chat-mention-card-self">You</p>
        ) : (
          onMessage && (
            <button
              type="button"
              className="chat-mention-card-message"
              disabled={pending}
              onClick={() => void message()}
            >
              {pending ? "Opening…" : "Message"}
            </button>
          )
        )}
        {error && (
          <p className="chat-action-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingPortal root={anchor.closest<HTMLElement>(".live-scene") ?? undefined}>
      {drawer ? (
        <FloatingOverlay lockScroll className="chat-actions-overlay">
          {panel}
        </FloatingOverlay>
      ) : (
        panel
      )}
    </FloatingPortal>
  );
}
