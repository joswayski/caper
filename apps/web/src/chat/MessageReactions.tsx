import { useEffect, useRef, useState } from "react";
import { autoUpdate, flip, FloatingPortal, offset, shift, useDismiss, useFloating, useFocus, useHover, useInteractions, useRole } from "@floating-ui/react";
import { SmilePlus } from "lucide-react";
import type { ChatMessage, ChatReaction } from "./types.ts";
import { emojiAsset, emojiCode } from "./emoji.ts";
import { preloadReactionPicker } from "./MessageActions.tsx";
import { cachedReactors, emojiLabel, emojiName, fallbackSummary, loadReactors, reactionSummary, reactorName } from "./reactors.ts";

export interface ReactionSave {
  emoji: string;
  active: boolean;
  error?: string;
}

const HOLD_MS = 500;

function ReactionChip({ message, reaction, channelId, authorId, canReact, onReact, onShowReactors }: {
  message: ChatMessage;
  reaction: ChatReaction;
  channelId?: string;
  authorId?: string;
  canReact: boolean;
  onReact: (messageId: string, emoji: string, active: boolean) => Promise<void>;
  onShowReactors: (emoji: string, anchor: HTMLElement) => void;
}) {
  const { emoji, authorIds } = reaction;
  const mine = !!authorId && authorIds.includes(authorId);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState<string>();
  const [summary, setSummary] = useState<string>();
  const hold = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number }>(undefined);
  const held = useRef(false);
  const { refs, floatingStyles, context } = useFloating({
    open, onOpenChange: setOpen, placement: "top", strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12 })], whileElementsMounted: autoUpdate,
  });
  // Mouse hover and keyboard focus only; touch uses press-and-hold instead.
  const { getReferenceProps, getFloatingProps } = useInteractions([
    useHover(context, { delay: { open: 400, close: 0 }, mouseOnly: true }),
    useFocus(context, { visibleOnly: true }),
    useDismiss(context),
    useRole(context, { role: "tooltip" }),
  ]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const label = () => emojiLabel(emoji, name);
    const names = (list = channelId ? cachedReactors(channelId, message.id, message.reactionSeq) : undefined) =>
      list?.reactions.find((item) => item.emoji === emoji)?.authors.map((author) => ({ id: author.id, name: reactorName(author) }));
    const known = names();
    setSummary(known ? reactionSummary(known, authorId, label()) : fallbackSummary(reaction, authorId, label()));
    void emojiName(emoji).then((found) => { if (active && found !== name) setName(found); });
    if (!known && channelId) {
      loadReactors(channelId, message.id, message.reactionSeq).then((list) => {
        const loaded = names(list);
        if (active && loaded) setSummary(reactionSummary(loaded, authorId, label()));
      }, () => {});
    }
    return () => { active = false; };
  }, [open, name, emoji, reaction, channelId, message.id, message.reactionSeq, authorId]);

  const cancelHold = () => { clearTimeout(hold.current?.timer); hold.current = undefined; };
  const show = (anchor: HTMLElement) => { setOpen(false); onShowReactors(emoji, anchor); };

  return <>
    <button type="button" ref={refs.setReference} className="chat-reaction" aria-pressed={mine} aria-disabled={!canReact}
      aria-label={`${emoji}, ${authorIds.length} ${authorIds.length === 1 ? "reaction" : "reactions"}${mine ? ", including you" : ""}`}
      aria-description="Press and hold, or right-click, to see who reacted"
      {...getReferenceProps({
        onClick: () => {
          if (held.current) { held.current = false; return; }
          if (canReact) void onReact(message.id, emoji, !mine);
        },
        onPointerDown: (event) => {
          cancelHold();
          held.current = false;
          if (event.pointerType === "mouse" || !event.isPrimary) return;
          const anchor = event.currentTarget as HTMLElement;
          hold.current = { x: event.clientX, y: event.clientY, timer: setTimeout(() => { held.current = true; show(anchor); }, HOLD_MS) };
        },
        onPointerMove: (event) => {
          const current = hold.current;
          if (current && Math.hypot(event.clientX - current.x, event.clientY - current.y) > 10) cancelHold();
        },
        onPointerUp: cancelHold,
        onPointerCancel: cancelHold,
        onContextMenu: (event) => {
          // Right-click with a mouse; some touch browsers also send this on hold.
          event.preventDefault();
          if (held.current) return;
          cancelHold();
          show(event.currentTarget as HTMLElement);
        },
      })}>
      <img src={emojiAsset(emojiCode(emoji))} width={18} height={18} alt={emoji} loading="lazy" />
      <span>{authorIds.length}</span>
    </button>
    {open && summary && <FloatingPortal>
      <div className="chat-reaction-tooltip" ref={refs.setFloating} style={floatingStyles} {...getFloatingProps()}>
        <img src={emojiAsset(emojiCode(emoji))} width={36} height={36} alt="" />
        <span>{summary}</span>
      </div>
    </FloatingPortal>}
  </>;
}

export default function MessageReactions({ message, channelId, authorId, readOnly = false, save, pickerOpen, onReact, onOpenPicker, onShowReactors, onDismissError }: {
  message: ChatMessage;
  channelId?: string;
  authorId?: string;
  readOnly?: boolean;
  save?: ReactionSave;
  pickerOpen: boolean;
  onReact: (messageId: string, emoji: string, active: boolean) => Promise<void>;
  onOpenPicker: (anchor: HTMLButtonElement) => void;
  onShowReactors: (emoji: string, anchor: HTMLElement) => void;
  onDismissError: () => void;
}) {
  const canReact = !!authorId && !readOnly;

  return <>
    <button type="button" className="chat-add-reaction" onClick={(event) => onOpenPicker(event.currentTarget)}
      onMouseEnter={() => { if (canReact) preloadReactionPicker(); }} onFocus={preloadReactionPicker}
      disabled={!canReact} aria-label="Add reaction" title="Add reaction" aria-haspopup="dialog" aria-expanded={pickerOpen}>
      <SmilePlus size={14} aria-hidden="true" />
    </button>
    {!!message.reactions?.length && <div className="chat-reactions" aria-label="Reactions">
      {message.reactions.map((reaction) => <ReactionChip key={reaction.emoji} message={message} reaction={reaction} channelId={channelId}
        authorId={authorId} canReact={canReact} onReact={onReact} onShowReactors={onShowReactors} />)}
    </div>}
    {save?.error && <div className="chat-send-status chat-send-error" role="alert">
      <span>{save.error}</span><button type="button" disabled={!canReact} onClick={() => void onReact(message.id, save.emoji, save.active)}>Retry reaction</button>
      <button type="button" onClick={onDismissError}>Dismiss</button>
    </div>}
  </>;
}
