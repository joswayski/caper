import { useEffect, useId, useRef, useState, type RefObject } from "react";
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
import GifButton, { GifPicker, useKlipy } from "./GifButton.tsx";
import type { ChatGif } from "./klipy.ts";

/**
 * Mobile composers share one Emojis / GIFs / Stickers drawer. Desktop keeps
 * its separate controls. The `:name:` shortcut stays available in the text.
 * Inside another floating card, register both in one `FloatingTree` so picking
 * an emoji is not an outside press for the card.
 */
export default function EmojiButton({
  disabled,
  onSelect,
  onOpenChange,
  returnFocus,
  onSendGif,
  gifDisabled,
}: {
  disabled?: boolean;
  onSelect: (emoji: string) => void;
  onOpenChange?: (open: boolean) => void;
  /** The composer that Escape, the close button or an outside press returns typing to. */
  returnFocus?: RefObject<HTMLTextAreaElement | null>;
  onSendGif?: (gif: ChatGif, query: string) => void;
  gifDisabled?: boolean;
}) {
  const [open, setOpenState] = useState(false);
  const [drawer, setDrawer] = useState(false);
  useEffect(() => {
    const query = window.matchMedia("(max-width: 760px), (pointer: coarse)");
    const update = () => setDrawer(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
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
    <FloatingFocusManager context={context} returnFocus={returnFocus ?? false}>
      <div
        ref={refs.setFloating}
        className={`chat-reaction-picker${drawer ? " chat-reaction-picker-drawer" : ""}`}
        style={drawer ? undefined : floatingStyles}
        aria-label={drawer ? "Emojis, GIFs and stickers" : "Insert emoji"}
        {...getFloatingProps()}
      >
        {drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-reaction-picker-heading">
          <strong>{drawer ? "Emojis, GIFs & stickers" : "Insert emoji"}</strong>
          <button type="button" aria-label="Close emoji picker" onClick={() => setOpen(false)}>
            <X size={16} />
          </button>
        </div>
        <div className="chat-reaction-picker-body">
          {drawer ? (
            <ComposerPicker
              onSelect={(emoji) => {
                setOpen(false);
                onSelect(emoji);
              }}
              gifDisabled={disabled || gifDisabled}
              onSendGif={
                onSendGif &&
                ((gif, query) => {
                  setOpen(false);
                  onSendGif(gif, query);
                })
              }
            />
          ) : (
            <ReactionPicker
              onSelect={(emoji) => {
                setOpen(false);
                onSelect(emoji);
              }}
            />
          )}
        </div>
      </div>
    </FloatingFocusManager>
  );

  return (
    <>
      {!drawer && onSendGif && (
        <GifButton
          disabled={disabled || gifDisabled}
          onSelect={onSendGif}
          onOpenChange={onOpenChange}
          returnFocus={returnFocus}
        />
      )}
      <FloatingNode id={nodeId}>
        <button
          ref={refs.setReference}
          type="button"
          className="chat-composer-tool"
          aria-label={drawer ? "Emojis, GIFs and stickers" : "Insert emoji"}
          title={drawer ? "Emojis, GIFs and stickers" : "Emoji"}
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
    </>
  );
}

const tabs = [
  { id: "emoji", label: "Emojis" },
  { id: "gif", label: "GIFs" },
  { id: "sticker", label: "Stickers" },
] as const;

/** Content only: all three tabs share the same dialog and focus boundary. */
function ComposerPicker({
  onSelect,
  onSendGif,
  gifDisabled,
}: {
  onSelect: (emoji: string) => void;
  onSendGif?: (gif: ChatGif, query: string) => void;
  gifDisabled?: boolean;
}) {
  const config = useKlipy();
  const [tab, setTab] = useState<(typeof tabs)[number]["id"]>("emoji");
  const id = useId();
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  return (
    <div className="chat-composer-picker">
      <div className="chat-gif-tabs chat-composer-picker-tabs" role="tablist" aria-label="Emojis, GIFs and stickers">
        {tabs.map((item, index) => (
          <button
            key={item.id}
            ref={(button) => {
              buttons.current[index] = button;
            }}
            type="button"
            role="tab"
            id={`${id}-${item.id}`}
            aria-controls={`${id}-panel`}
            aria-selected={tab === item.id}
            tabIndex={tab === item.id ? 0 : -1}
            onClick={() => setTab(item.id)}
            onKeyDown={(event) => {
              const next =
                event.key === "ArrowRight"
                  ? (index + 1) % tabs.length
                  : event.key === "ArrowLeft"
                    ? (index + tabs.length - 1) % tabs.length
                    : event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? tabs.length - 1
                        : undefined;
              if (next === undefined) return;
              event.preventDefault();
              setTab(tabs[next].id);
              buttons.current[next]?.focus();
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div className="chat-reaction-picker-body" id={`${id}-panel`} role="tabpanel" aria-labelledby={`${id}-${tab}`}>
        {tab === "emoji" ? (
          <ReactionPicker autoFocus={false} onSelect={onSelect} />
        ) : config && onSendGif ? (
          <GifPicker
            key={tab}
            kind={tab}
            config={config}
            ads
            autoFocus={false}
            disabled={gifDisabled}
            onSelect={onSendGif}
          />
        ) : (
          <p role="status">
            {tab === "gif" ? "GIFs" : "Stickers"} are unavailable right now. You can still insert emojis.
          </p>
        )}
      </div>
    </div>
  );
}
