import { appGateway } from "../gateway/client.ts";
import { getRememberedAccount, rememberSavedAccount } from "./client.ts";
import { liveUpdate, receiveProfile, withProfile, type SpaceUpdate } from "./profiles.ts";
import { notificationSettings, type NotificationSettings } from "../spaces/client.ts";
import { receiveAccountNotificationSettings } from "../spaces/notifications.ts";

/** Routes one `profiles` event: your own profile updates the signed-in account
 * (as a save would); anyone else's joins the live profile store; a space update
 * goes to `onSpace`; your notification settings (Do Not Disturb included) saved
 * on another device update this tab's. Unknown or malformed events are ignored. */
export function applyLiveUpdate(value: unknown, onSpace: (space: SpaceUpdate) => void) {
  const event = value as { type?: unknown; settings?: unknown } | null;
  if (event?.type === "notifications.updated" && event.settings && typeof event.settings === "object")
    return receiveAccountNotificationSettings(
      notificationSettings({ ...(event.settings as NotificationSettings), overrides: [] }),
    );
  const update = liveUpdate(value);
  if (!update) return;
  if (update.type === "space.updated") return onSpace(update.space);
  const account = getRememberedAccount();
  if (account?.id === update.person.id) rememberSavedAccount(withProfile(account, update.person));
  else receiveProfile(update.person);
}

/** Live name, username, picture and space changes while signed in. The gateway
 * re-subscribes after every reconnect; there is no replay, so what the page
 * refetches then is the repair. An older gateway refuses the kind: the error
 * just means live updates are unavailable, and nothing retries or disconnects. */
export function watchLiveUpdates(onSpace: (space: SpaceUpdate) => void) {
  const subscription = appGateway().subscribe(
    { kind: "profiles" },
    { event: (value) => applyLiveUpdate(value, onSpace), error: () => undefined },
  );
  void subscription.ready.catch(() => undefined);
  return () => subscription.unsubscribe();
}
