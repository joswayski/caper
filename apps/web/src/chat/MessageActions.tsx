import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
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
import { Ban, Copy, Forward, Hash, History, Pencil, Pin, PinOff, SmilePlus, Trash2, Users, X } from "lucide-react";
import { Reply, Thread } from "../../../../shared/message-icons.ts";
import type { ChatMessage } from "./types.ts";
import { emojiAsset, emojiCode, preloadEmojiImages } from "./emoji.ts";
import ReactionPicker from "./ReactionPicker.tsx";

export interface MessageActionTarget {
  messageId: string;
  anchor: HTMLElement;
  anchorRect?: DOMRect;
  mode: "actions" | "emoji";
  drawer: boolean;
  inThread: boolean;
}

const quickReactions = ["👍", "❤️", "😂", "🎉", "👀"];

export function preloadReactionPicker() {
  void preloadEmojiImages();
}

export default function MessageActions({
  message,
  target,
  authorId,
  canReact,
  canPin,
  canForward = false,
  canEdit,
  canDelete = false,
  pinning,
  onReact,
  onPin,
  onForward,
  onClose,
  onCopied,
  onViewReactions,
  onReply,
  onReplyMessage,
  onEdit,
  onDelete,
  onHistory,
  block,
}: {
  message: ChatMessage;
  target: MessageActionTarget;
  authorId?: string;
  canReact: boolean;
  canPin: boolean;
  canForward?: boolean;
  canEdit: boolean;
  canDelete?: boolean;
  pinning: boolean;
  onReact: (messageId: string, emoji: string, active: boolean) => Promise<void>;
  onPin: (messageId: string, active: boolean) => Promise<void>;
  onForward?: () => void;
  onClose: () => void;
  onCopied: (status: string) => void;
  onViewReactions: (emoji: string) => void;
  onReply?: () => void;
  onReplyMessage?: () => void;
  onEdit: () => void;
  /** The caller confirms before deleting. */
  onDelete?: () => void;
  onHistory: () => void;
  /** Another signed-in author. Blocking is confirmed by the caller; unblocking is immediate. */
  block?: { blocked: boolean; name: string; onBlock: () => void; onUnblock: () => Promise<void> };
}) {
  const [mode, setMode] = useState(target.mode);
  const [copyError, setCopyError] = useState<string>();
  const returnFocus = useRef(target.anchor);
  const [restoreFocus, setRestoreFocus] = useState(true);
  const reply = (action: () => void) => {
    // Reply hands focus to the composer. Menu cleanup must not take it back.
    flushSync(() => setRestoreFocus(false));
    action();
  };
  const { refs, floatingStyles, context } = useFloating({
    open: true,
    onOpenChange: (open) => {
      if (!open) onClose();
    },
    placement: "bottom-end",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context)]);
  useEffect(() => {
    refs.setReference(target.anchor);
    // The preview/list handoff can replace a row while its picker is open.
    // Preserve the trigger's last position instead of anchoring at (0, 0).
    refs.setPositionReference({
      contextElement: target.anchor,
      getBoundingClientRect: () =>
        target.anchor.isConnected
          ? target.anchor.getBoundingClientRect()
          : (target.anchorRect ?? target.anchor.getBoundingClientRect()),
    });
  }, [target.anchor, target.anchorRect, refs.setReference, refs.setPositionReference]);
  useEffect(() => {
    if (!canReact && mode === "emoji") onClose();
  }, [canReact, mode, onClose]);

  const react = (emoji: string, active: boolean) => {
    if (!canReact) return;
    onClose();
    void onReact(message.id, emoji, active);
  };
  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      onCopied(`${label} copied.`);
      onClose();
    } catch {
      setCopyError("Couldn’t copy. Check clipboard permission and try again.");
    }
  };

  const panel = (
    <FloatingFocusManager context={context} returnFocus={restoreFocus ? returnFocus : false}>
      <div
        className={
          mode === "actions"
            ? `chat-message-actions${target.drawer ? " chat-message-actions-drawer" : ""}`
            : `chat-reaction-picker${target.drawer ? " chat-reaction-picker-drawer" : ""}`
        }
        ref={refs.setFloating}
        style={target.drawer ? undefined : floatingStyles}
        aria-label={mode === "actions" ? "Message actions" : "Add a reaction"}
        {...getFloatingProps()}
      >
        {target.drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-reaction-picker-heading">
          <strong>{mode === "actions" ? "Message actions" : "Add a reaction"}</strong>
          <button
            type="button"
            aria-label={mode === "actions" ? "Close message actions" : "Close emoji picker"}
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </div>
        {mode === "actions" ? (
          <div className="chat-message-actions-body">
            {canReact && (
              <div className="chat-quick-reactions" aria-label="Quick reactions">
                {quickReactions.map((emoji) => {
                  const mine =
                    !!authorId &&
                    !!message.reactions?.some(
                      (reaction) => reaction.emoji === emoji && reaction.authorIds.includes(authorId),
                    );
                  return (
                    <button
                      type="button"
                      key={emoji}
                      aria-label={`React with ${emoji}`}
                      aria-pressed={mine}
                      onClick={() => react(emoji, !mine)}
                    >
                      <img src={emojiAsset(emojiCode(emoji))} width={20} height={20} alt="" />
                    </button>
                  );
                })}
                <button
                  type="button"
                  aria-label="Add reaction"
                  onMouseEnter={preloadReactionPicker}
                  onFocus={preloadReactionPicker}
                  onClick={() => setMode("emoji")}
                >
                  <SmilePlus size={16} aria-hidden="true" />
                </button>
              </div>
            )}
            <div className="chat-copy-actions">
              {onReplyMessage && (
                <button type="button" onClick={() => reply(onReplyMessage)}>
                  <Reply size={16} aria-hidden="true" />
                  Reply
                </button>
              )}
              {onReply && (
                <button type="button" onClick={() => reply(onReply)}>
                  <Thread size={16} aria-hidden="true" />
                  Reply in thread
                </button>
              )}
              {canForward && (
                <button type="button" onClick={onForward}>
                  <Forward size={16} aria-hidden="true" />
                  Forward message
                </button>
              )}
              {canEdit && (
                <button type="button" onClick={onEdit}>
                  <Pencil size={16} aria-hidden="true" />
                  Edit message
                </button>
              )}
              {!message.forward && (message.revision ?? 1) > 1 && (
                <button type="button" onClick={onHistory}>
                  <History size={16} aria-hidden="true" />
                  Message history
                </button>
              )}
              {!!message.reactions?.length && (
                <button type="button" onClick={() => onViewReactions(message.reactions![0].emoji)}>
                  <Users size={16} aria-hidden="true" />
                  View reactions
                </button>
              )}
              {canPin && (
                <button
                  type="button"
                  disabled={pinning}
                  onClick={() => {
                    onClose();
                    void onPin(message.id, !message.pin);
                  }}
                >
                  {message.pin ? <PinOff size={16} aria-hidden="true" /> : <Pin size={16} aria-hidden="true" />}
                  {pinning ? "Saving…" : message.pin ? "Unpin message" : "Pin message"}
                </button>
              )}
              {message.content.text && (
                <button type="button" onClick={() => void copy(message.content.text, "Text")}>
                  <Copy size={16} aria-hidden="true" />
                  Copy text
                </button>
              )}
              <button type="button" onClick={() => void copy(message.id, "Message ID")}>
                <Hash size={16} aria-hidden="true" />
                Copy message ID
              </button>
              {canDelete && (
                <button type="button" className="message-action-danger" onClick={onDelete}>
                  <Trash2 size={16} aria-hidden="true" />
                  Delete message
                </button>
              )}
              {block && (
                <button
                  type="button"
                  className={block.blocked ? undefined : "message-action-danger"}
                  onClick={() => {
                    onClose();
                    if (block.blocked)
                      void block.onUnblock().catch(() => onCopied(`Couldn’t unblock ${block.name}. Try again.`));
                    else block.onBlock();
                  }}
                >
                  <Ban size={16} aria-hidden="true" />
                  {block.blocked ? `Unblock ${block.name}` : `Block ${block.name}`}
                </button>
              )}
            </div>
            {copyError && (
              <p className="chat-action-error" role="alert">
                {copyError}
              </p>
            )}
          </div>
        ) : (
          <div className="chat-reaction-picker-body">
            <ReactionPicker autoFocus={!target.drawer} onSelect={(emoji) => react(emoji, true)} />
          </div>
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
