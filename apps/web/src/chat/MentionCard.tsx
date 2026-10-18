import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Ellipsis, MessageCircle, X } from "lucide-react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingNode,
  FloatingOverlay,
  FloatingPortal,
  FloatingTree,
  offset,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import Avatar from "../components/Avatar";
import type { MentionCardPerson } from "./mentions.ts";
import type { ChatAuthor } from "./types.ts";
import { addSpaceMember, type Space } from "../spaces/client.ts";
import { friendlyError } from "../spaces/errors.ts";
import { unblock, useBlockedIds } from "../spaces/blocks.ts";
import { getRememberedAccount, subscribeAccount, withLatestProfile } from "../account/client.ts";
import { getProfiles, getServerProfiles, subscribeProfiles } from "../account/profiles.ts";
import { canEditProfile, editProfile } from "../account/profile-editor.ts";
import ProfileComposer from "./ProfileComposer.tsx";

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

type MentionCardProps = {
  target: MentionCardTarget;
  onClose: () => void;
  onMessage?: (username: string) => Promise<void>;
  /** The account host uses the existing block confirmation. */
  onBlock?: (author: ChatAuthor) => void;
  /** Only non-demo spaces owned by the signed-in account. */
  spaces?: Space[];
};

/** Who a mention points at, with a composer to message them directly. */
export default function MentionCard(props: MentionCardProps) {
  // The composer's emoji picker is a child floating element of this card.
  return (
    <FloatingTree>
      <MentionCardPanel {...props} />
    </FloatingTree>
  );
}

function MentionCardPanel({ target, onClose, onMessage, onBlock, spaces = [] }: MentionCardProps) {
  const { person: snapshot, anchor, drawer } = target;
  const account = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const profiles = useSyncExternalStore(subscribeProfiles, getProfiles, getServerProfiles);
  const person = withLatestProfile(snapshot, account, profiles);
  const blocked = useBlockedIds().has(person.id);
  const [pending, setPending] = useState<string>();
  const [error, setError] = useState<string>();
  const [choosingSpace, setChoosingSpace] = useState(false);
  /** Space ID -> "Invited" or "Member", so a sent or redundant invitation can't be retried. */
  const [invited, setInvited] = useState<ReadonlyMap<string, string>>(() => new Map());
  const [status, setStatus] = useState<string>();
  const [composerState, setComposerState] = useState({ picking: false, sending: false });
  const canMessage = !person.self && !blocked && !!onMessage && !!person.username;
  const canInvite = !person.self && !blocked && !!person.username && spaces.length > 0;
  const returnFocus = useRef(anchor);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open: true,
    onOpenChange: (open) => {
      if (!open && !composerState.sending && !pending) onClose();
    },
    placement: "bottom-start",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([
    useDismiss(context, { outsidePress: !composerState.picking && !composerState.sending && !pending }),
    useRole(context),
  ]);
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
      initialFocus={target.focusOnOpen === false ? -1 : canMessage && !drawer ? composerInput : 0}
      closeOnFocusOut={false}
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
            <Avatar person={person} name={person.displayName ?? person.username ?? title} />
          </i>
          <div>
            <strong>{title}</strong>
            {person.displayName && person.username && <span>@{person.username}</span>}
          </div>
          <div className="chat-profile-header-actions">
            {!person.self && (onBlock || canInvite) && (
              <ProfileMenu
                disabled={!!pending || composerState.sending}
                label={pending === "unblock" ? "Unblocking…" : blocked ? "Unblock" : "Block"}
                onInvite={canInvite ? () => setChoosingSpace(!choosingSpace) : undefined}
                onAction={
                  onBlock
                    ? () => {
                        if (blocked) void unblockPerson();
                        else
                          onBlock({
                            id: person.id,
                            name: person.displayName ?? person.username ?? "Profile",
                            avatarId: person.avatarId,
                            avatarAsset: person.avatarAsset,
                            avatarUrl: person.avatarUrl,
                            isGuest: false,
                          });
                      }
                    : undefined
                }
              />
            )}
            {!person.self && !blocked && onMessage && person.username && (
              <button
                type="button"
                aria-label="Open conversation"
                title={pending === "message" ? "Opening…" : "Open conversation"}
                disabled={!!pending || composerState.sending}
                onClick={() => void message()}
              >
                <MessageCircle size={18} aria-hidden="true" />
              </button>
            )}
            <button
              type="button"
              aria-label="Close profile"
              title="Close"
              disabled={composerState.sending || !!pending}
              onClick={onClose}
            >
              <X size={18} aria-hidden="true" />
            </button>
          </div>
        </div>
        {person.self && (
          <>
            <p className="chat-mention-card-self">You</p>
            {canEditProfile() && (
              <button
                type="button"
                className="chat-mention-card-message chat-mention-card-edit"
                onClick={() => {
                  onClose();
                  editProfile();
                }}
              >
                Edit profile
              </button>
            )}
          </>
        )}
        {!person.self && !blocked && choosingSpace && (
          <ul className="chat-profile-spaces" aria-label="Spaces you own">
            {spaces.map((space) => (
              <li key={space.id}>
                <button
                  type="button"
                  className="chat-mention-card-action"
                  disabled={!!pending || composerState.sending || invited.has(space.id)}
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
        {canMessage && (
          <ProfileComposer
            personId={person.id}
            username={person.username!}
            inputRef={composerInput}
            onSent={() => void message()}
            onStateChange={setComposerState}
          />
        )}
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingNode id={nodeId}>
      <FloatingPortal root={anchor.closest<HTMLElement>(".live-scene") ?? undefined}>
        {drawer ? (
          <FloatingOverlay lockScroll className="chat-actions-overlay">
            {panel}
          </FloatingOverlay>
        ) : (
          panel
        )}
      </FloatingPortal>
    </FloatingNode>
  );
}

/** A child menu keeps secondary actions out of the profile's primary action row. */
function ProfileMenu({
  disabled,
  label,
  onAction,
  onInvite,
}: {
  disabled: boolean;
  label: string;
  onAction?: () => void;
  onInvite?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open,
    onOpenChange: setOpen,
    placement: "bottom-end",
    strategy: "fixed",
    middleware: [offset(4), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps, getItemProps } = useInteractions([
    useClick(context),
    useDismiss(context, { bubbles: false }),
    useRole(context, { role: "menu" }),
  ]);
  return (
    <FloatingNode id={nodeId}>
      <button
        ref={refs.setReference}
        type="button"
        aria-label="More profile actions"
        title="More profile actions"
        disabled={disabled}
        {...getReferenceProps()}
      >
        <Ellipsis size={18} aria-hidden="true" />
      </button>
      {open && (
        <FloatingPortal>
          <FloatingFocusManager context={context}>
            <div ref={refs.setFloating} style={floatingStyles} className="chat-profile-menu" {...getFloatingProps()}>
              {onInvite && (
                <button
                  type="button"
                  disabled={disabled}
                  {...getItemProps({
                    onClick: () => {
                      setOpen(false);
                      onInvite();
                    },
                  })}
                >
                  Invite to space
                </button>
              )}
              {onAction && (
                <button
                  type="button"
                  disabled={disabled}
                  {...getItemProps({
                    onClick: () => {
                      setOpen(false);
                      onAction();
                    },
                  })}
                >
                  {label}
                </button>
              )}
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </FloatingNode>
  );
}
