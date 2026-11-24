// Reply and Mark as read on Caper's browser notifications, and opening a
// conversation from one. There is no fetch handler: this worker never sits
// between the page and the network, and it caches nothing.
"use strict";

const MESSAGE = /^[A-Za-z0-9]{15}$/;

/** Only the account that showed the notification may act on it. */
async function signedIn(accountId) {
  const response = await fetch("/api/account/me", { credentials: "same-origin", cache: "no-store" });
  if (!response.ok) return false;
  const account = await response.json().catch(() => null);
  return account?.id === accountId;
}

async function act(path, body) {
  const request = () =>
    fetch(path, {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body ?? {}),
    });
  // One retry for a dropped connection; the reply's client ID makes it safe.
  const response = await request().catch(() => request());
  if (!response.ok) throw new Error(String(response.status));
}

async function windows() {
  return self.clients.matchAll({ type: "window", includeUncontrolled: true });
}

/** One tab shows the conversation in place (keeping a call and drafts); with none open, a new one loads it. */
async function open(location) {
  const all = await windows();
  const chosen = all.find((client) => client.visibilityState === "visible") ?? all[0];
  if (!chosen) return self.clients.openWindow(location);
  chosen.postMessage({ type: "caper.notification.open", location });
  return chosen.focus();
}

async function handle(event) {
  const notification = event.notification;
  const data = notification.data ?? {};
  notification.close();
  const action = event.action;
  if (action !== "reply" && action !== "read") return open(data.location);
  if (typeof data.messageId !== "string" || !MESSAGE.test(data.messageId)) return open(data.location);
  const reply = typeof event.reply === "string" ? event.reply.trim() : "";
  // Browsers without inline replies send the Reply button with no text: open the conversation instead.
  if (action === "reply" && !reply) return open(data.location);
  try {
    if (!(await signedIn(data.accountId))) return open(data.location);
    const base = `/api/notifications/messages/${data.messageId}`;
    if (action === "read") await act(`${base}/read`);
    else await act(`${base}/reply`, { clientMessageId: crypto.randomUUID(), text: reply });
    // Open tabs refresh unread counts.
    for (const client of await windows()) client.postMessage({ type: "caper.notification.acted" });
  } catch {
    if (action === "reply")
      await self.registration.showNotification(notification.title, {
        body: `Couldn’t send “${reply}”. Open Caper to try again.`,
        tag: `${notification.tag}:failed`,
        icon: notification.icon,
        data: { location: data.location },
      });
  }
}

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
self.addEventListener("notificationclick", (event) => event.waitUntil(handle(event)));
