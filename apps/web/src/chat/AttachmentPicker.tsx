import { useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingNode,
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

/** Shared choice order and native filters; Files deliberately has no filter. */
const attachmentChoices = [
  { label: "Photos and videos", accept: "image/*,video/*" },
  { label: "Files", accept: undefined },
] as const;

export default function AttachmentPicker({
  disabled,
  title,
  onAdd,
  onPickingChange,
  onRestoreFocus,
}: {
  disabled: boolean;
  title: string;
  onAdd: (files: File[]) => void;
  onPickingChange?: (picking: boolean) => void;
  onRestoreFocus?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  // Capture when the native picker opens, not when it eventually returns.
  const pickerAdd = useRef<typeof onAdd | undefined>(undefined);
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open: open && !disabled,
    onOpenChange: setOpen,
    placement: "top-start",
    strategy: "fixed",
    middleware: [offset(4), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps, getItemProps } = useInteractions([
    useClick(context),
    useDismiss(context, { bubbles: false }),
    useRole(context, { role: "menu" }),
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
  return (
    <FloatingNode id={nodeId}>
      <button
        ref={refs.setReference}
        type="button"
        className="chat-attach"
        aria-label="Attach files"
        title={title}
        disabled={disabled}
        {...getReferenceProps()}
      >
        <Plus size={18} aria-hidden="true" />
      </button>
      {attachmentChoices.map((choice, index) => (
        <input
          key={choice.label}
          ref={(element) => {
            inputs.current[index] = element;
            element?.addEventListener("cancel", finish);
            return () => element?.removeEventListener("cancel", finish);
          }}
          type="file"
          multiple
          hidden
          accept={choice.accept}
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
          <FloatingFocusManager context={context} returnFocus={!picking}>
            <div ref={refs.setFloating} style={floatingStyles} className="chat-profile-menu" {...getFloatingProps()}>
              {attachmentChoices.map((choice, index) => (
                <button
                  key={choice.label}
                  ref={(element) => {
                    items.current[index] = element;
                  }}
                  type="button"
                  role="menuitem"
                  tabIndex={activeIndex === index ? 0 : -1}
                  {...getItemProps({
                    onClick: () => {
                      pickerAdd.current = onAdd;
                      setPicking(true);
                      setOpen(false);
                      inputs.current[index]?.click();
                    },
                  })}
                >
                  {choice.label}
                </button>
              ))}
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </FloatingNode>
  );
}
