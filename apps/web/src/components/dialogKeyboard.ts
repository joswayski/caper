import type { KeyboardEvent } from "react";

/** Route Escape through the dialog's existing cancel policy on key-down. */
export function dialogKeyDown(event: KeyboardEvent<HTMLDialogElement>) {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  // Native select popups own their first Escape; the browser knows when to
  // cancel the dialog after the popup has closed.
  if (event.target instanceof HTMLSelectElement) return;
  event.preventDefault();
  event.stopPropagation();
  // Cancelling an IME composition must not also dismiss its editor.
  if (event.nativeEvent.isComposing) return;
  // Reuse onCancel for keyboard and platform dismissal, including busy guards.
  event.currentTarget.dispatchEvent(new Event("cancel", { cancelable: true }));
}
