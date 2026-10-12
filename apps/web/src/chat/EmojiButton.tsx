import { useState } from "react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingNode,
  FloatingOverlay,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { Smile, X } from "lucide-react";
import { preloadReactionPicker } from "./MessageActions.tsx";
import ReactionPicker from "./ReactionPicker.tsx";

/**
 * A composer's emoji control: a picker above the button, or a bottom drawer on
 * touch layouts. The `:name:` shortcut stays available in the text itself.
 * Inside another floating card, register both in one `FloatingTree` so picking
 * an emoji is not an outside press for the card.
 */
export default function EmojiButton({
  disabled,
  onSelect,
  onOpenChange,
}: {
  disabled?: boolean;
  onSelect: (emoji: string) => void;
  onOpenChange?: (open: boolean) => void;
}) {
  const [open, setOpenState] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const setOpen = (next: boolean) => {
    setOpenState(next);
    onOpenChange?.(next);
  };
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open,
    onOpenChange: setOpen,
    placement: "top-end",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps } = useInteractions([
    // Escape and outside presses close only the picker, not a card that hosts it.
    useDismiss(context, { bubbles: false }),
    useRole(context),
  ]);

  const panel = (
    <FloatingFocusManager context={context} returnFocus={false}>
      <div
        ref={refs.setFloating}
        className={`chat-reaction-picker${drawer ? " chat-reaction-picker-drawer" : ""}`}
        style={drawer ? undefined : floatingStyles}
        aria-label="Insert emoji"
        {...getFloatingProps()}
      >
        {drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-reaction-picker-heading">
          <strong>Insert emoji</strong>
          <button type="button" aria-label="Close emoji picker" onClick={() => setOpen(false)}>
            <X size={16} />
          </button>
        </div>
        <div className="chat-reaction-picker-body">
          <ReactionPicker
            onSelect={(emoji) => {
              setOpen(false);
              onSelect(emoji);
            }}
          />
        </div>
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingNode id={nodeId}>
      <button
        ref={refs.setReference}
        type="button"
        className="chat-composer-tool"
        aria-label="Insert emoji"
        title="Emoji"
        aria-expanded={open}
        disabled={disabled}
        onMouseEnter={preloadReactionPicker}
        onFocus={preloadReactionPicker}
        // Keep the textarea focused (and its caret visible) until the picker opens.
        onMouseDown={(event) => event.preventDefault()}
        {...getReferenceProps({
          onClick: () => {
            setDrawer(window.matchMedia("(max-width: 760px), (pointer: coarse)").matches);
            setOpen(!open);
          },
        })}
      >
        <Smile size={18} aria-hidden="true" />
      </button>
      {open && (
        <FloatingPortal root={refs.domReference.current?.closest<HTMLElement>(".live-scene") ?? undefined}>
          {drawer ? (
            <FloatingOverlay lockScroll className="chat-actions-overlay">
              {panel}
            </FloatingOverlay>
          ) : (
            panel
          )}
        </FloatingPortal>
      )}
    </FloatingNode>
  );
}
