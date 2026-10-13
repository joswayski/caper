import { useRef, type MouseEvent, type PointerEvent } from "react";

/** Whether a modal dialog's own event landed on its backdrop rather than its box. */
function onBackdrop(event: MouseEvent<HTMLDialogElement>) {
  const rect = event.currentTarget.getBoundingClientRect();
  return (
    event.target === event.currentTarget &&
    (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom)
  );
}

/**
 * Close a modal `<dialog>` on a click that both starts and ends on its backdrop,
 * so selecting text inside and releasing outside does not dismiss it.
 */
export function useBackdropDismiss(onClose: () => void, enabled = true) {
  const press = useRef(false);
  return {
    onPointerDown: (event: PointerEvent<HTMLDialogElement>) => {
      press.current = onBackdrop(event);
    },
    onClick: (event: MouseEvent<HTMLDialogElement>) => {
      if (enabled && press.current && onBackdrop(event)) onClose();
      press.current = false;
    },
  };
}
