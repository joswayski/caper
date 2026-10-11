import { useEffect, useRef, useState, useSyncExternalStore } from "react";
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
import type { ChatAuthor } from "./types.ts";
import { addSpaceMember, type Space } from "../spaces/client.ts";
import { friendlyError } from "../spaces/errors.ts";
import { unblock, useBlockedIds } from "../spaces/blocks.ts";
import { getRememberedAccount, subscribeAccount, withAccountProfile } from "../account/client.ts";

export interface MentionCardTarget {
  person: MentionCardPerson;
  anchor: HTMLElement;
  /** Phones and touch layouts get the bottom sheet used by message actions. */
  drawer: boolean;
  /** A mouse hover preview must not take keyboard focus. */
  focusOnOpen?: boolean;
}

const redundantInvitation: Record<string, string | undefined> = {
  "user already in space": "Member",
  "user already invited": "Invited",
};

/** Who a mention points at, with Message to open or start a DM. */
export default function MentionCard({
  target,
  onClose,
  onMessage,
  onBlock,
  spaces = [],
}: {
  target: MentionCardTarget;
  onClose: () => void;
  onMessage?: (username: string) => Promise<void>;
  /** The account host uses the existing block confirmation. */
  onBlock?: (author: ChatAuthor) => void;
  /** Only non-demo spaces owned by the signed-in account. */
  spaces?: Space[];
}) {
  const { person: snapshot, anchor, drawer } = target;
  const account = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const person = withAccountProfile(snapshot, account);
  const blocked = useBlockedIds().has(person.id);
  const [pending, setPending] = useState<string>();
  const [error, setError] = useState<string>();
  const [choosingSpace, setChoosingSpace] = useState(false);
  /** Space ID -> "Invited" or "Member", so a sent or redundant invitation can't be retried. */
  const [invited, setInvited] = useState<ReadonlyMap<string, string>>(() => new Map());
  const [status, setStatus] = useState<string>();
  const returnFocus = useRef(anchor);
  const { refs, floatingStyles, context } = useFloating({
    open: true,
    onOpenChange: (open) => {
      if (!open) onClose();
    },
    placement: "bottom-start",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context)]);
  useEffect(() => {
    refs.setReference(anchor);
  }, [anchor, refs.setReference]);
  const title = person.displayName ?? (person.username ? `@${person.username}` : "Profile");

  const message = async () => {
    if (!onMessage || !person.username || blocked || pending) return;
    setPending("message");
    setError(undefined);
    try {
      await onMessage(person.username);
      onClose();
    } catch (reason) {
      setError(friendlyError(reason));
      setPending(undefined);
    }
  };

  const invite = async (space: Space) => {
    if (!person.username || blocked || pending || invited.has(space.id)) return;
    setPending(space.id);
    setError(undefined);
    setStatus(undefined);
    try {
      await addSpaceMember(space.id, person.username);
      setInvited((current) => new Map(current).set(space.id, "Invited"));
      setStatus(`Invitation sent to ${space.name}.`);
    } catch (reason) {
      // Redundant invitations still mark the row so it isn't retried.
      const already = reason instanceof Error ? redundantInvitation[reason.message] : undefined;
      if (already) setInvited((current) => new Map(current).set(space.id, already));
      setError(friendlyError(reason));
    } finally {
      setPending(undefined);
    }
  };

  const unblockPerson = async () => {
    if (pending) return;
    setPending("unblock");
    setError(undefined);
    try {
      await unblock(person.id);
      setStatus("Unblocked.");
    } catch (reason) {
      setError(friendlyError(reason));
    } finally {
      setPending(undefined);
    }
  };

  const panel = (
    <FloatingFocusManager
      context={context}
      returnFocus={returnFocus}
      initialFocus={target.focusOnOpen === false ? -1 : 0}
    >
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
            <Avatar avatarId={person.avatarId} name={person.displayName ?? person.username ?? title} />
          </i>
          <div>
            <strong>{title}</strong>
            {person.displayName && person.username && <span>@{person.username}</span>}
          </div>
        </div>
        {person.self ? (
          <p className="chat-mention-card-self">You</p>
        ) : (
          <>
            {!blocked && onMessage && person.username && (
              <button
                type="button"
                className="chat-mention-card-message"
                disabled={!!pending}
                onClick={() => void message()}
              >
                {pending === "message" ? "Opening…" : "Message"}
              </button>
            )}
            {!blocked && person.username && spaces.length > 0 && (
              <>
                <button
                  type="button"
                  className="chat-mention-card-action"
                  aria-expanded={choosingSpace}
                  disabled={!!pending}
                  onClick={() => setChoosingSpace(!choosingSpace)}
                >
                  Invite to space
                </button>
                {choosingSpace && (
                  <ul className="chat-profile-spaces" aria-label="Spaces you own">
                    {spaces.map((space) => (
                      <li key={space.id}>
                        <button
                          type="button"
                          className="chat-mention-card-action"
                          disabled={!!pending || invited.has(space.id)}
                          onClick={() => void invite(space)}
                        >
                          <span>{space.name}</span>
                          {pending === space.id ? (
                            <small>Sending…</small>
                          ) : (
                            invited.has(space.id) && <small>{invited.get(space.id)}</small>
                          )}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </>
            )}
            {onBlock && (
              <button
                type="button"
                className="chat-mention-card-action chat-profile-block"
                disabled={!!pending}
                onClick={() => {
                  if (blocked) void unblockPerson();
                  else
                    onBlock({
                      id: person.id,
                      name: person.displayName ?? person.username ?? "Profile",
                      avatarId: person.avatarId,
                      isGuest: false,
                    });
                }}
              >
                {pending === "unblock" ? "Unblocking…" : blocked ? "Unblock" : "Block"}
              </button>
            )}
          </>
        )}
        {status && (
          <p className="chat-profile-status" role="status">
            {status}
          </p>
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
