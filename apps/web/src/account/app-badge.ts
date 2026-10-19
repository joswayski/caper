/** Installed-app badging is optional; the tab title remains useful without it. */
let writes = Promise.resolve();
let revision = 0;

export function setApplicationBadge(count: number) {
  if (typeof document === "undefined" || !Number.isSafeInteger(count) || count < 0) return;
  document.title = count ? `(${count}) Caper` : "Caper";
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

/** Only acknowledged reads invalidate the authoritative unread snapshot. */
export function unreadChanged() {
  if (typeof window !== "undefined") window.dispatchEvent(new Event(UNREAD_CHANGED));
}
