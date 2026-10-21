import { useEffect, useRef, useState } from "react";
import { Camera, FileText, FileUp, Images, Plus, X, type LucideIcon } from "lucide-react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingNode,
  FloatingOverlay,
  FloatingPortal,
  offset,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useInteractions,
  useListNavigation,
  useRole,
} from "@floating-ui/react";
import { attachCapacity } from "./attach.ts";

interface AttachChoice {
  id: "camera" | "media" | "files";
  label: string;
  Icon: LucideIcon;
  accept?: string;
  capture?: "environment";
}

/** Phones keep a photo library apart from their files, so touch layouts offer
 * both (and the camera, where the browser can open one). Elsewhere one
 * unfiltered "Upload a file" covers everything, as on the desktop clients. */
const touchChoices: AttachChoice[] = [
  { id: "camera", label: "Camera", Icon: Camera, accept: "image/*,video/*", capture: "environment" },
  { id: "media", label: "Photos and videos", Icon: Images, accept: "image/*,video/*" },
  { id: "files", label: "Files", Icon: FileText },
];
const desktopChoices: AttachChoice[] = [{ id: "files", label: "Upload a file", Icon: FileUp }];

export const touchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;
const hasCamera = () => window.matchMedia("(pointer: coarse)").matches;

/**
 * The composer's attach drawer: a bottom sheet on touch layouts and a menu
 * above the button elsewhere. It stays openable at the limit, where it says
 * so and choices nudge that line instead of opening a picker that couldn't
 * add anything.
 */
export default function AttachmentPicker({
  disabled,
  title,
  count,
  onAdd,
  onPickingChange,
  onRestoreFocus,
}: {
  /** Uploads unavailable or the conversation is read-only (not the file limit). */
  disabled: boolean;
  title: string;
  /** Files already in this draft. */
  count: number;
  onAdd: (files: File[]) => void;
  onPickingChange?: (picking: boolean) => void;
  onRestoreFocus?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const [choices, setChoices] = useState(desktopChoices);
  const [picking, setPicking] = useState(false);
  const [nudges, setNudges] = useState(0);
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const inputs = useRef<Partial<Record<AttachChoice["id"], HTMLInputElement | null>>>({});
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  // Capture when the native picker opens, not when it eventually returns.
  const pickerAdd = useRef<typeof onAdd | undefined>(undefined);
  const capacity = attachCapacity(count);
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open: open && !disabled,
    onOpenChange: (next) => {
      if (next) {
        const touch = touchLayout();
        setDrawer(touch);
        setChoices(touch ? touchChoices.filter((choice) => choice.id !== "camera" || hasCamera()) : desktopChoices);
        setNudges(0);
      }
      setOpen(next);
    },
    placement: "top-start",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps, getItemProps } = useInteractions([
    useClick(context),
    useDismiss(context, { bubbles: false }),
    useRole(context, { role: "dialog" }),
    useListNavigation(context, { listRef: items, activeIndex, onNavigate: setActiveIndex, loop: true }),
  ]);
  useEffect(() => {
    onPickingChange?.((open && !disabled) || picking);
  }, [open, disabled, picking, onPickingChange]);
  const finish = () => {
    pickerAdd.current = undefined;
    setPicking(false);
    requestAnimationFrame(() => {
      if (onRestoreFocus) onRestoreFocus();
      else (refs.domReference.current as HTMLButtonElement | null)?.focus();
    });
  };
  const choose = (choice: AttachChoice) => {
    if (capacity.full) {
      // Restart the nudge on every attempt, and buzz phones that support it.
      setNudges((value) => value + 1);
      navigator.vibrate?.(30);
      return;
    }
    pickerAdd.current = onAdd;
    setPicking(true);
    setOpen(false);
    inputs.current[choice.id]?.click();
  };

  const panel = (
    <FloatingFocusManager context={context} returnFocus={!picking} initialFocus={drawer ? -1 : 0}>
      <div
        ref={refs.setFloating}
        style={drawer ? undefined : floatingStyles}
        className={`chat-attach-drawer${drawer ? " chat-attach-drawer-sheet" : ""}`}
        aria-label="Attach"
        data-full={capacity.full || undefined}
        {...getFloatingProps()}
      >
        {drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        {(drawer || capacity.full) && (
          <div className="chat-attach-heading">
            {drawer && <strong>Attach</strong>}
            {capacity.hint && (
              <p
                key={nudges}
                className="chat-attach-hint"
                data-nudge={nudges > 0 || undefined}
                role="status"
                aria-live="polite"
              >
                {capacity.hint}
              </p>
            )}
            {drawer && (
              <button type="button" className="chat-attach-close" aria-label="Close" onClick={() => setOpen(false)}>
                <X size={16} aria-hidden="true" />
              </button>
            )}
          </div>
        )}
        <div className="chat-attach-options">
          {choices.map((choice, index) => (
            <button
              key={choice.id}
              ref={(element) => {
                items.current[index] = element;
              }}
              type="button"
              aria-disabled={capacity.full || undefined}
              tabIndex={activeIndex === index || (activeIndex === null && index === 0) ? 0 : -1}
              {...getItemProps({ onClick: () => choose(choice) })}
            >
              <choice.Icon size={drawer ? 22 : 18} aria-hidden="true" />
              <span>{choice.label}</span>
            </button>
          ))}
        </div>
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingNode id={nodeId}>
      <button
        ref={refs.setReference}
        type="button"
        className="chat-attach"
        aria-label="Attach files"
        aria-expanded={open && !disabled}
        title={title}
        disabled={disabled}
        {...getReferenceProps()}
      >
        <Plus size={18} aria-hidden="true" />
      </button>
      {touchChoices.map((choice) => (
        <input
          key={choice.id}
          ref={(element) => {
            inputs.current[choice.id] = element;
            element?.addEventListener("cancel", finish);
            return () => element?.removeEventListener("cancel", finish);
          }}
          type="file"
          multiple={!choice.capture}
          hidden
          accept={choice.accept}
          capture={choice.capture}
          onChange={(event) => {
            const files = [...(event.target.files ?? [])];
            event.target.value = "";
            const add = pickerAdd.current;
            finish();
            if (files.length) add?.(files);
          }}
        />
      ))}
      {open && !disabled && (
        <FloatingPortal>
          {drawer ? (
            <FloatingOverlay lockScroll className="chat-actions-overlay chat-attach-overlay">
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

/** The same sheet as the composer's touch drawer, for choosing one picture
 * (a profile picture or space icon): Camera, Photos and Files, images only. */
export function PictureSourceSheet({
  open,
  title,
  onChoose,
  onClose,
}: {
  /** Its inputs stay mounted while closed, so a picker opened from it can still answer. */
  open: boolean;
  title: string;
  onChoose: (file: File) => void;
  onClose: () => void;
}) {
  const choices: AttachChoice[] = [
    { id: "camera", label: "Camera", Icon: Camera, accept: "image/*", capture: "environment" },
    { id: "media", label: "Photos", Icon: Images, accept: "image/*" },
    { id: "files", label: "Files", Icon: FileText, accept: "image/*" },
  ].filter((choice) => choice.id !== "camera" || hasCamera()) as AttachChoice[];
  const inputs = useRef<Partial<Record<AttachChoice["id"], HTMLInputElement | null>>>({});
  const { refs, context } = useFloating({ open, onOpenChange: (next) => !next && onClose() });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context, { role: "dialog" })]);
  return (
    <>
      {choices.map((choice) => (
        <input
          key={choice.id}
          ref={(element) => {
            inputs.current[choice.id] = element;
            // Canceling the native picker must not dismiss any enclosing dialogs.
            const cancel = (event: Event) => event.stopPropagation();
            element?.addEventListener("cancel", cancel);
            return () => element?.removeEventListener("cancel", cancel);
          }}
          type="file"
          hidden
          accept={choice.accept}
          capture={choice.capture}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) onChoose(file);
          }}
        />
      ))}
      {open && (
        <FloatingPortal>
          <FloatingOverlay lockScroll className="chat-actions-overlay chat-attach-overlay">
            <FloatingFocusManager context={context} initialFocus={-1}>
              <div
                ref={refs.setFloating}
                className="chat-attach-drawer chat-attach-drawer-sheet"
                aria-label={title}
                {...getFloatingProps()}
              >
                <div className="chat-drawer-handle" aria-hidden="true" />
                <div className="chat-attach-heading">
                  <strong>{title}</strong>
                  <button type="button" className="chat-attach-close" aria-label="Close" onClick={onClose}>
                    <X size={16} aria-hidden="true" />
                  </button>
                </div>
                <div className="chat-attach-options">
                  {choices.map((choice) => (
                    <button
                      key={choice.id}
                      type="button"
                      onClick={() => {
                        // The sheet closes as the picker opens, as the composer's does.
                        inputs.current[choice.id]?.click();
                        onClose();
                      }}
                    >
                      <choice.Icon size={22} aria-hidden="true" />
                      <span>{choice.label}</span>
                    </button>
                  ))}
                </div>
              </div>
            </FloatingFocusManager>
          </FloatingOverlay>
        </FloatingPortal>
      )}
    </>
  );
}
