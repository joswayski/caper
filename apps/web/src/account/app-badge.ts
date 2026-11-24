/** Installed-app badging is optional; the tab title remains useful without it. */
let writes = Promise.resolve();
let revision = 0;
let badge = 0;
let conversation: string | undefined;

function writeTitle() {
  document.title = `${badge ? `(${badge}) ` : ""}${conversation ? `${conversation} · ` : ""}Caper`;
}

/** Names the open conversation in the tab title, such as "#general · Weekend crew". */
export function setConversationTitle(label: string | undefined) {
  if (typeof document === "undefined") return;
  conversation = label;
  writeTitle();
}

export function setApplicationBadge(count: number) {
  if (typeof document === "undefined" || !Number.isSafeInteger(count) || count < 0) return;
  badge = count;
  writeTitle();
  const attempt = ++revision;
  // Serialize writes and skip superseded ones, including reads racing logout.
  writes = writes.then(async () => {
    if (attempt !== revision) return;
    try {
      const badges = navigator as Navigator & {
        setAppBadge?: (count: number) => Promise<void>;
        clearAppBadge?: () => Promise<void>;
      };
      if (count) await badges.setAppBadge?.(count);
      else await badges.clearAppBadge?.();
    } catch {
      // Permission, install state and OS support may prevent an icon badge.
    }
  });
}

export const UNREAD_CHANGED = "caper:unread-changed";

let changes = 0;

/** Only acknowledged reads invalidate the authoritative unread snapshot. */
export function unreadChanged() {
  changes++;
  if (typeof window !== "undefined") window.dispatchEvent(new Event(UNREAD_CHANGED));
}

/** Counts `unreadChanged` calls, so a snapshot can tell whether it predates one. */
export function unreadChanges() {
  return changes;
}
