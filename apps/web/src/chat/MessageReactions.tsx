import { useEffect, useRef, useState, type ComponentType } from "react";
import { autoUpdate, flip, FloatingFocusManager, FloatingPortal, offset, shift, useClick, useDismiss, useFloating, useInteractions, useRole } from "@floating-ui/react";
import { SmilePlus, X } from "lucide-react";
import type { ChatMessage } from "./types.ts";
import { emojiAsset, emojiCode } from "./emoji.ts";

export default function MessageReactions({ message, authorId, onReact }: {
  message: ChatMessage;
  authorId?: string;
  onReact: (messageId: string, emoji: string, active: boolean) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [Picker, setPicker] = useState<ComponentType<{ onSelect: (emoji: string) => void }>>();
  const [loadError, setLoadError] = useState(false);
  const [saving, setSaving] = useState(false);
  const busy = useRef(false);
  const [failure, setFailure] = useState<{ emoji: string; active: boolean; error: string }>();
  const { refs, floatingStyles, context } = useFloating({
    open, onOpenChange: setOpen, placement: "bottom-end", strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })], whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps } = useInteractions([useClick(context), useDismiss(context), useRole(context)]);
  useEffect(() => {
    if (!open || Picker) return;
    let current = true;
    setLoadError(false);
    import("./ReactionPicker.tsx").then((module) => { if (current) setPicker(() => module.default); }, () => { if (current) setLoadError(true); });
    return () => { current = false; };
  }, [open, Picker]);

  const react = async (emoji: string, active: boolean) => {
    if (busy.current) return;
    busy.current = true;
    setSaving(true);
    setFailure(undefined);
    setOpen(false);
    try { await onReact(message.id, emoji, active); }
    catch (error) { setFailure({ emoji, active, error: error instanceof Error ? error.message : "Reaction could not be saved." }); }
    finally { busy.current = false; setSaving(false); }
  };

  return <>
    <button type="button" className="chat-add-reaction" ref={refs.setReference}
      disabled={!authorId || saving} aria-label="Add reaction" title="Add reaction" {...getReferenceProps()}>
      <SmilePlus size={18} aria-hidden="true" />
    </button>
    {!!message.reactions?.length && <div className="chat-reactions" aria-label="Reactions">
      {message.reactions.map(({ emoji, authorIds }) => {
        const mine = !!authorId && authorIds.includes(authorId);
        return <button type="button" key={emoji} className="chat-reaction" aria-pressed={mine}
          disabled={!authorId || saving} aria-label={`${emoji}, ${authorIds.length} ${authorIds.length === 1 ? "reaction" : "reactions"}${mine ? ", including you" : ""}`}
          title={mine ? "Remove your reaction" : "Add your reaction"} onClick={() => void react(emoji, !mine)}>
          <img src={emojiAsset(emojiCode(emoji))} width={18} height={18} alt={emoji} loading="lazy" />
          <span>{authorIds.length}</span>
        </button>;
      })}
    </div>}
    {saving && <div className="chat-send-status" role="status">Saving reaction…</div>}
    {failure && <div className="chat-send-status chat-send-error" role="alert">
      <span>{failure.error}</span><button type="button" onClick={() => void react(failure.emoji, failure.active)}>Retry reaction</button>
      <button type="button" onClick={() => setFailure(undefined)}>Dismiss</button>
    </div>}
    {open && <FloatingPortal root={refs.domReference.current?.closest<HTMLElement>(".live-scene") ?? undefined}><FloatingFocusManager context={context}>
      <div className="chat-reaction-picker" ref={refs.setFloating} style={floatingStyles} aria-label="Choose a reaction" {...getFloatingProps()}>
        <div className="chat-reaction-picker-heading"><strong>Add a reaction</strong><button type="button" aria-label="Close emoji picker" onClick={() => setOpen(false)}><X size={18} /></button></div>
        <div className="chat-reaction-picker-body">
          {Picker ? <Picker onSelect={(emoji) => void react(emoji, true)} />
            : <p role={loadError ? "alert" : "status"}>{loadError ? "Couldn’t load emoji. Close and try again." : "Loading emoji…"}</p>}
        </div>
      </div>
    </FloatingFocusManager></FloatingPortal>}
  </>;
}
