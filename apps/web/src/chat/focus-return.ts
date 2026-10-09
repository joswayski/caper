/**
 * Remembers where focus was when a message dialog opened and returns it on close.
 * A menu item that opened the dialog is gone by then, so fall back to the
 * message's actions button rather than leaving focus on the page body.
 */
export function focusReturn(messageKey: string) {
  const opener = document.activeElement;
  return () => {
    const target =
      opener instanceof HTMLElement && opener !== document.body && opener.isConnected
        ? opener
        : document.querySelector<HTMLElement>(
            `[data-message-key="${CSS.escape(messageKey)}"] .chat-message-actions-trigger`,
          );
    target?.focus();
  };
}
