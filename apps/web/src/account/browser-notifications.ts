import { appGateway } from "../gateway/client";
import { avatarUrl } from "./avatar";
import { notificationImage } from "./notification-image";
import { pictureUrl } from "./pictures";
import { setAccountFeedLive } from "./shell-poll";

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

/**
 * Reply and Mark as read need a service worker's persistent notifications
 * and a browser that shows actions (Chromium; inline text on Chrome and Edge).
 * Elsewhere notifications stay as they were: a click opens Caper.
 */
let worker: Promise<ServiceWorkerRegistration | undefined> | undefined;
function notificationWorker() {
  if (typeof Notification === "undefined" || !("actions" in Notification.prototype) || !navigator.serviceWorker)
    return Promise.resolve(undefined);
  worker ??= navigator.serviceWorker
    .register("/notifications-sw.js")
    .then(() =>
      Promise.race([
        navigator.serviceWorker.ready,
        new Promise<undefined>((resolve) => setTimeout(() => resolve(undefined), 5_000)),
      ]),
    )
    .catch(() => undefined);
  return worker;
}

type ActionOptions = NotificationOptions & {
  image?: string;
  actions?: { action: string; title: string; type?: "button" | "text"; placeholder?: string }[];
};

/** Messages can be answered or read from the notification; invitations, requests and voice cannot. */
function actionable(event: MessageNotification) {
  return (
    typeof event.messageId === "string" &&
    /^[A-Za-z0-9]{15}$/.test(event.messageId) &&
    !event.notificationId &&
    !["space.invitation", "channel.invitation", "direct.request", "voice.started"].includes(event.kind ?? "")
  );
}

export interface MessageNotification {
  type: "notification.created";
  seq: string;
  kind?: string;
  messageId?: string;
  notificationId?: string;
  title: string;
  body: string;
  createdAt: string;
  senderAvatarId?: number | null;
  senderAvatarUrl?: string;
  attachmentUrl?: string;
  attachmentContentType?: string;
  attachmentSize?: number;
  conversationId?: string;
  spaceId?: string;
  channelId?: string;
  /** The thread a reply belongs to. */
  threadRootId?: string;
  /** Another device of this account was used more recently and alerts instead. */
  quiet?: boolean;
}

export function notificationLocation(event: MessageNotification) {
  const id = /^[A-Za-z0-9]{12}$/;
  const message = /^[A-Za-z0-9]{15}$/;
  if (event.kind === "voice.started") {
    if (event.spaceId && event.channelId && id.test(event.spaceId) && id.test(event.channelId) && !event.conversationId)
      return `/spaces?space=${event.spaceId}&channel=${event.channelId}`;
    return;
  }
  // The feedback team visits the conversation in place, at the report's thread.
  const visit = event.conversationId ?? event.channelId;
  if (event.kind === "feedback" && visit && id.test(visit) && event.messageId && message.test(event.messageId)) {
    const root = event.threadRootId && message.test(event.threadRootId) ? event.threadRootId : event.messageId;
    return `/spaces?feedback=${visit}&message=${event.messageId}&thread=${root}`;
  }
  if (event.kind === "direct.request") {
    if (event.conversationId && id.test(event.conversationId) && !event.spaceId && !event.channelId)
      return `/spaces?dm=${event.conversationId}`;
    return;
  }
  if (
    event.kind === "space.invitation" &&
    event.spaceId &&
    id.test(event.spaceId) &&
    !event.channelId &&
    !event.conversationId
  )
    return `/spaces?space=${event.spaceId}`;
  if (
    event.kind === "channel.invitation" &&
    event.spaceId &&
    event.channelId &&
    id.test(event.spaceId) &&
    id.test(event.channelId) &&
    !event.conversationId
  )
    return `/spaces?space=${event.spaceId}&invite=${event.channelId}`;
  if (event.kind === "space.invitation" || event.kind === "channel.invitation") return;
  if (event.conversationId && id.test(event.conversationId)) return `/spaces?dm=${event.conversationId}`;
  if (event.spaceId && event.channelId && id.test(event.spaceId) && id.test(event.channelId))
    return `/spaces?space=${event.spaceId}&channel=${event.channelId}`;
}

/**
 * Account-wide, not tied to the current channel's chat subscription. A click
 * offers its location to `open`, which returns true when it showed it in place
 * (keeping a voice call and drafts); otherwise the page loads the location.
 */
export function watchBrowserNotifications(
  accountId: string,
  showing: () => string | undefined,
  /** Called with the kinds of the notifications that arrived, about a second after the first. */
  onActivity?: (kinds: ReadonlySet<string>) => void,
  open?: (location: string) => boolean,
) {
  let active = true;
  const downloads = new AbortController();
  const tagPrefix = `caper:${accountId}:`;
  // The worker hands a notification click to one tab, and reports Reply and Mark as read.
  const fromWorker = (message: MessageEvent) => {
    const data = message.data as { type?: unknown; location?: unknown } | null;
    if (!active) return;
    if (data?.type === "caper.notification.acted") onActivity?.(new Set());
    if (
      data?.type === "caper.notification.open" &&
      typeof data.location === "string" &&
      data.location.startsWith("/spaces?")
    ) {
      window.focus();
      if (!open?.(data.location)) window.location.assign(data.location);
    }
  };
  navigator.serviceWorker?.addEventListener("message", fromWorker);
  const workerNotifications = async (tag?: string) => {
    const registration = await notificationWorker();
    const shown = (await registration?.getNotifications(tag ? { tag } : undefined)) ?? [];
    return shown.filter((notification) => notification.tag.startsWith(tagPrefix));
  };
  const displayed = new Map<Notification, string | undefined>();
  const identities = new WeakMap<Notification, string>();
  // Messages deleted after they notified: closed if shown, never shown if late.
  const retracted = new Set<string>();
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
  // A burst of notifications coalesces into one unread refresh about a second later.
  let activityTimer: ReturnType<typeof setTimeout> | undefined;
  let activityKinds = new Set<string>();
  const activity = (kind: string | undefined) => {
    if (!onActivity) return;
    if (kind) activityKinds.add(kind);
    if (activityTimer !== undefined) return;
    activityTimer = setTimeout(() => {
      activityTimer = undefined;
      const kinds = activityKinds;
      activityKinds = new Set();
      if (active) onActivity(kinds);
    }, 1_000);
  };
  const receive = async (value: unknown) => {
    const event = value as MessageNotification;
    const invitation = event?.kind === "space.invitation" || event?.kind === "channel.invitation";
    const request = event?.kind === "direct.request";
    const voice = event?.kind === "voice.started";
    const identity =
      invitation || request || voice
        ? typeof event.notificationId === "string" && /^[1-9][0-9]*$/.test(event.notificationId)
          ? `${voice ? "voice" : request ? "request" : "invitation"}:${event.notificationId}`
          : undefined
        : event?.messageId;
    if (
      !event ||
      event.type !== "notification.created" ||
      typeof identity !== "string" ||
      typeof event.title !== "string" ||
      typeof event.body !== "string" ||
      retracted.has(identity)
    )
      return;
    activity(event.kind);
    const location = notificationLocation(event);
    const fresh = () => {
      const age = Date.now() - Date.parse(event.createdAt);
      return Number.isFinite(age) && age >= -30_000 && age <= 120_000;
    };
    if (!location || event.quiet === true || !fresh()) return;
    const subject = invitation || request ? identity : (event.conversationId ?? event.channelId);
    const allowed = () => active && browserNotificationPermission() === "granted" && browserNotificationsEnabled();
    const reading = () => {
      visibility();
      let visible = document.visibilityState === "visible" && document.hasFocus() && showing() === subject;
      try {
        const other = JSON.parse(localStorage.getItem(visibleKey) ?? "null");
        visible ||= other?.conversation === subject && Date.now() - other.at < 3_000;
      } catch {
        /* Storage may be unavailable. */
      }
      return visible;
    };
    // Persist only bounded opaque IDs, never previews. The lock serializes
    // competing tabs so only one claims this message; the image download and
    // display happen after release so a slow original never delays later alerts.
    const claim = () => {
      if (!allowed()) return false;
      const visible = reading();
      const key = `caper.notificationSeen.${accountId}`;
      try {
        const seen: string[] = JSON.parse(localStorage.getItem(key) ?? "[]");
        if (Array.isArray(seen) && seen.includes(identity)) return false;
        localStorage.setItem(key, JSON.stringify([...(Array.isArray(seen) ? seen : []), identity].slice(-128)));
      } catch {
        /* Private browsing may disallow storage; the OS tag still groups duplicates. */
      }
      return !visible;
    };
    const claimed = navigator.locks
      ? await navigator.locks.request(`caper.notifications.${accountId}`, claim)
      : claim();
    if (!claimed) return;
    const image = event.attachmentUrl ? await notificationImage(event, downloads.signal) : undefined;
    if (!allowed() || !fresh() || reading() || retracted.has(identity)) {
      if (image) URL.revokeObjectURL(image);
      return;
    }
    try {
      const options: ActionOptions = {
        body: event.body,
        icon: pictureUrl(event.senderAvatarUrl) ?? avatarUrl(event.senderAvatarId),
        tag: `${tagPrefix}${identity}`,
        ...(image ? { image } : {}),
      };
      const registration = actionable(event) ? await notificationWorker() : undefined;
      if (registration && active) {
        const persistent: ActionOptions = {
          ...options,
          data: { location, messageId: event.messageId, accountId },
          actions: [
            { action: "reply", type: "text", title: "Reply", placeholder: "Reply" },
            { action: "read", title: "Mark as read" },
          ],
        };
        // TypeScript's DOM types have not caught up with notification actions.
        await registration.showNotification(event.title, persistent as NotificationOptions);
        // The browser read the image when it showed the notification.
        if (image) URL.revokeObjectURL(image);
        if (retracted.has(identity)) for (const shown of await workerNotifications(options.tag)) shown.close();
        return;
      }
      const notification = new Notification(event.title, options);
      displayed.set(notification, image);
      identities.set(notification, identity);
      notification.onclose = () => {
        displayed.delete(notification);
        if (image) URL.revokeObjectURL(image);
      };
      notification.onclick = () => {
        notification.close();
        if (!active) return;
        window.focus();
        if (!open?.(location)) window.location.assign(location);
      };
    } catch {
      if (image) URL.revokeObjectURL(image);
      /* Some mobile browsers require a service worker; no closed-browser push yet. */
    }
  };
  const subscription = appGateway().subscribe(
    { kind: "notifications" },
    {
      event: (value) => {
        void receive(value);
      },
      // Polls slow down while this feed reports activity itself.
      status: (online) => {
        if (active) setAccountFeedLive(online);
      },
    },
  );
  void subscription.ready.catch(() => undefined);
  // `notification.retracted` arrives on the profiles feed; other tabs close their own.
  const retractions = appGateway().subscribe(
    { kind: "profiles" },
    {
      event: (value) => {
        const event = value as { type?: unknown; messageId?: unknown };
        if (event?.type !== "notification.retracted" || typeof event.messageId !== "string" || !event.messageId) return;
        retracted.add(event.messageId);
        if (retracted.size > 256) retracted.delete(retracted.values().next().value!);
        for (const notification of displayed.keys())
          if (identities.get(notification) === event.messageId) notification.close();
        void workerNotifications(`${tagPrefix}${event.messageId}`).then((shown) => shown.forEach((n) => n.close()));
      },
      error: () => undefined,
    },
  );
  void retractions.ready.catch(() => undefined);
  return () => {
    active = false;
    setAccountFeedLive(false);
    retractions.unsubscribe();
    clearTimeout(activityTimer);
    downloads.abort();
    subscription.unsubscribe();
    navigator.serviceWorker?.removeEventListener("message", fromWorker);
    // Sign-out or an account switch: this account's notifications go, actions and all.
    void workerNotifications().then((shown) => shown.forEach((notification) => notification.close()));
    clearInterval(visibleTimer);
    window.removeEventListener("focus", visibility);
    window.removeEventListener("blur", visibility);
    document.removeEventListener("visibilitychange", visibility);
    try {
      if (JSON.parse(localStorage.getItem(visibleKey) ?? "null")?.tab === tab) localStorage.removeItem(visibleKey);
    } catch {
      /* Storage may be unavailable. */
    }
    for (const [notification, image] of displayed) {
      notification.close();
      if (image) URL.revokeObjectURL(image);
    }
  };
}
