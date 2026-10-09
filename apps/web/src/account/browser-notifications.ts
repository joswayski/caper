import { appGateway } from "../gateway/client";
import { avatarUrl } from "./avatar";

const CHOICE = "caper.browserNotifications";

export function browserNotificationPermission(): NotificationPermission | "unsupported" {
  return typeof Notification === "undefined" ? "unsupported" : Notification.permission;
}

export function browserNotificationsEnabled() {
  try {
    return localStorage.getItem(CHOICE) !== "off";
  } catch {
    return true;
  }
}

export function setBrowserNotificationsEnabled(on: boolean) {
  try {
    localStorage.setItem(CHOICE, on ? "on" : "off");
  } catch {
    /* Permission still governs delivery. */
  }
}

export interface MessageNotification {
  type: "notification.created";
  seq: string;
  messageId: string;
  title: string;
  body: string;
  createdAt: string;
  senderAvatarId?: number | null;
  conversationId?: string;
  spaceId?: string;
  channelId?: string;
}

export function notificationLocation(event: MessageNotification) {
  const id = /^[A-Za-z0-9]{12}$/;
  if (event.conversationId && id.test(event.conversationId)) return `/spaces?dm=${event.conversationId}`;
  if (event.spaceId && event.channelId && id.test(event.spaceId) && id.test(event.channelId))
    return `/spaces?space=${event.spaceId}&channel=${event.channelId}`;
}

/** Account-wide, not tied to the current channel's chat subscription. */
export function watchBrowserNotifications(accountId: string, showing: () => string | undefined) {
  let active = true;
  const displayed = new Set<Notification>();
  const visibleKey = `caper.notificationVisible.${accountId}`;
  const tab = crypto.randomUUID();
  // A background tab must also suppress the conversation visible in another tab.
  const visibility = () => {
    try {
      if (document.visibilityState === "visible" && document.hasFocus()) {
        localStorage.setItem(visibleKey, JSON.stringify({ tab, conversation: showing(), at: Date.now() }));
      } else if (JSON.parse(localStorage.getItem(visibleKey) ?? "null")?.tab === tab) {
        localStorage.removeItem(visibleKey);
      }
    } catch {
      /* Tab coordination is best effort when storage is unavailable. */
    }
  };
  visibility();
  const visibleTimer = setInterval(visibility, 1_000);
  window.addEventListener("focus", visibility);
  window.addEventListener("blur", visibility);
  document.addEventListener("visibilitychange", visibility);
  const receive = async (value: unknown) => {
    const event = value as MessageNotification;
    if (
      !event ||
      event.type !== "notification.created" ||
      typeof event.messageId !== "string" ||
      typeof event.title !== "string" ||
      typeof event.body !== "string"
    )
      return;
    const location = notificationLocation(event);
    const age = Date.now() - Date.parse(event.createdAt);
    if (!location || !Number.isFinite(age) || age < -30_000 || age > 120_000) return;
    const present = () => {
      if (!active || browserNotificationPermission() !== "granted" || !browserNotificationsEnabled()) return;
      visibility();
      let visible =
        document.visibilityState === "visible" &&
        document.hasFocus() &&
        showing() === (event.conversationId ?? event.channelId);
      // Persist only bounded opaque IDs, never previews. The lock serializes
      // competing tabs so only one presents this message.
      const key = `caper.notificationSeen.${accountId}`;
      try {
        const other = JSON.parse(localStorage.getItem(visibleKey) ?? "null");
        visible ||= other?.conversation === (event.conversationId ?? event.channelId) && Date.now() - other.at < 3_000;
        const seen: string[] = JSON.parse(localStorage.getItem(key) ?? "[]");
        if (Array.isArray(seen) && seen.includes(event.messageId)) return;
        localStorage.setItem(key, JSON.stringify([...(Array.isArray(seen) ? seen : []), event.messageId].slice(-128)));
      } catch {
        /* Private browsing may disallow storage; the OS tag still groups duplicates. */
      }
      if (visible) return;
      try {
        const notification = new Notification(event.title, {
          body: event.body,
          icon: avatarUrl(event.senderAvatarId),
          tag: `caper:${accountId}:${event.messageId}`,
        });
        displayed.add(notification);
        notification.onclose = () => displayed.delete(notification);
        notification.onclick = () => {
          notification.close();
          if (!active) return;
          window.focus();
          window.location.assign(location);
        };
      } catch {
        /* Some mobile browsers require a service worker; no closed-browser push yet. */
      }
    };
    if (navigator.locks) await navigator.locks.request(`caper.notifications.${accountId}`, present);
    else present();
  };
  const subscription = appGateway().subscribe(
    { kind: "notifications" },
    {
      event: (value) => {
        void receive(value);
      },
    },
  );
  void subscription.ready.catch(() => undefined);
  return () => {
    active = false;
    subscription.unsubscribe();
    clearInterval(visibleTimer);
    window.removeEventListener("focus", visibility);
    window.removeEventListener("blur", visibility);
    document.removeEventListener("visibilitychange", visibility);
    try {
      if (JSON.parse(localStorage.getItem(visibleKey) ?? "null")?.tab === tab) localStorage.removeItem(visibleKey);
    } catch {
      /* Storage may be unavailable. */
    }
    for (const notification of displayed) notification.close();
  };
}
