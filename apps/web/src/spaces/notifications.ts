import { useSyncExternalStore } from "react";
import {
  getNotificationSettings,
  setChannelNotifications,
  setDirectNotifications,
  setSpaceNotifications,
  updateNotificationSettings,
  type MobileNotifications,
  type MutedUntil,
  type NotificationChange,
  type NotificationLevel,
  type NotificationOverride,
  type NotificationSettings,
} from "./client.ts";

export const levelLabels: Record<NotificationLevel, string> = {
  all: "All messages",
  mentions: "Only @mentions",
  nothing: "Nothing",
};

export const mutePresets: ReadonlyArray<{ label: string; minutes?: number }> = [
  { label: "For 15 minutes", minutes: 15 },
  { label: "For 1 hour", minutes: 60 },
  { label: "For 8 hours", minutes: 8 * 60 },
  { label: "For 24 hours", minutes: 24 * 60 },
  { label: "Until I turn it back on" },
];

/** The `mutedUntil` a preset sends, in whole seconds like the API's own timestamps. */
export function muteUntil(minutes: number | undefined, now = Date.now()): MutedUntil {
  if (minutes === undefined) return "forever";
  return new Date(now + minutes * 60_000).toISOString().replace(/\.\d+Z$/, "Z");
}

/** Expired mutes read as not muted, as they do on the server. */
export function isMuted(mutedUntil: MutedUntil | undefined, now = Date.now()) {
  return mutedUntil === "forever" || (!!mutedUntil && Date.parse(mutedUntil) > now);
}

/** "Muted until 5:00 PM" in local time, with the date when it isn't today, or "Muted" with no end. */
export function muteLabel(mutedUntil: MutedUntil | undefined, now = new Date(), locale?: string) {
  if (!mutedUntil || !isMuted(mutedUntil, now.getTime())) return undefined;
  if (mutedUntil === "forever") return "Muted";
  const until = new Date(mutedUntil);
  const time: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
  if (until.toDateString() === now.toDateString()) return `Muted until ${until.toLocaleTimeString(locale, time)}`;
  const year = until.getFullYear() === now.getFullYear() ? undefined : "numeric";
  return `Muted until ${until.toLocaleString(locale, { month: "short", day: "numeric", year, ...time })}`;
}

export type NotificationScope = { spaceId: string; channelId?: string } | { conversationId: string };

export interface NotificationState {
  /** False until the first load, so menus never show a guessed level. */
  loaded: boolean;
  level: NotificationLevel;
  mobile: MobileNotifications;
  /** Keyed by `scopeKey`; scopes with nothing set are absent. */
  overrides: ReadonlyMap<string, NotificationOverride>;
}

export function scopeKey(scope: { spaceId?: string; channelId?: string; conversationId?: string }) {
  if (scope.conversationId) return `dm:${scope.conversationId}`;
  return scope.channelId ? `channel:${scope.channelId}` : `space:${scope.spaceId}`;
}

export function overrideFor(state: NotificationState, scope: NotificationScope) {
  return state.overrides.get(scopeKey(scope));
}

/** What "Default" means in a menu: a channel inherits its space's level, a space the account level. */
export function inheritedLevel(state: NotificationState, scope: NotificationScope) {
  if ("channelId" in scope && scope.channelId)
    return overrideFor(state, { spaceId: scope.spaceId })?.level ?? state.level;
  return state.level;
}

// One copy per tab, shared by the sidebar, every menu and the settings dialog.
const INITIAL: NotificationState = { loaded: false, level: "all", mobile: "whenInactive", overrides: new Map() };
let state = INITIAL;
const listeners = new Set<() => void>();
// Every local change bumps `revision`, so a slower response never undoes a newer choice.
let revision = 0;
let pending = 0;
let refreshing: Promise<void> | undefined;
let expiry: ReturnType<typeof setTimeout> | undefined;

function publish(next: NotificationState) {
  state = next;
  scheduleExpiry();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function put(overrides: Map<string, NotificationOverride>, key: string, override: NotificationOverride) {
  if (override.level === null && override.mutedUntil === null) overrides.delete(key);
  else overrides.set(key, override);
}

function withOverride(key: string, override: NotificationOverride) {
  const overrides = new Map(state.overrides);
  put(overrides, key, override);
  return { ...state, overrides };
}

function fromSettings(settings: NotificationSettings): NotificationState {
  return {
    loaded: true,
    level: settings.level,
    mobile: settings.mobile,
    overrides: new Map(settings.overrides.map((override) => [scopeKey(override), override])),
  };
}

// Re-render when the next timed mute ends, dropping it as the server would.
function scheduleExpiry() {
  clearTimeout(expiry);
  const now = Date.now();
  let next = Infinity;
  for (const { mutedUntil } of state.overrides.values()) {
    const end = mutedUntil && mutedUntil !== "forever" ? Date.parse(mutedUntil) : NaN;
    if (end > now) next = Math.min(next, end);
  }
  if (next !== Infinity) expiry = setTimeout(dropExpiredMutes, Math.min(next - now + 250, 2 ** 31 - 1));
}

function dropExpiredMutes() {
  const now = Date.now();
  const overrides = new Map(state.overrides);
  for (const [key, override] of state.overrides) {
    if (override.mutedUntil && !isMuted(override.mutedUntil, now))
      put(overrides, key, { ...override, mutedUntil: null });
  }
  publish({ ...state, overrides });
}

/** Loads after sign-in and whenever settings or a menu opens; ignored if a change was made meanwhile. */
export function refreshNotificationSettings() {
  refreshing ??= (async () => {
    const started = revision;
    try {
      const settings = await getNotificationSettings();
      if (started === revision && !pending) publish(fromSettings(settings));
    } finally {
      refreshing = undefined;
    }
  })();
  return refreshing;
}

/** Optimistic; on failure only the fields this change still owns go back, and the error is rethrown. */
export async function setNotificationLevel(level: NotificationLevel) {
  const previous = state.level;
  const mine = ++revision;
  pending++;
  publish({ ...state, level });
  try {
    const settings = await updateNotificationSettings({ level });
    // Overrides may have changes of their own in flight; take only the account fields.
    if (mine === revision) publish({ ...state, level: settings.level, mobile: settings.mobile });
  } catch (error) {
    if (state.level === level) publish({ ...state, level: previous });
    throw error;
  } finally {
    pending--;
  }
}

async function changeOverride(
  scope: NotificationScope,
  change: NotificationChange,
  send: () => Promise<NotificationOverride>,
) {
  const key = scopeKey(scope);
  const before: NotificationOverride = state.overrides.get(key) ?? { ...scope, level: null, mutedUntil: null };
  const optimistic = { ...before, ...change };
  const mine = ++revision;
  pending++;
  publish(withOverride(key, optimistic));
  try {
    const saved = await send();
    if (mine === revision) publish(withOverride(key, saved));
  } catch (error) {
    const reverted = { ...(state.overrides.get(key) ?? { ...scope, level: null, mutedUntil: null }) };
    if ("level" in change && reverted.level === optimistic.level) reverted.level = before.level;
    if ("mutedUntil" in change && reverted.mutedUntil === optimistic.mutedUntil)
      reverted.mutedUntil = before.mutedUntil;
    publish(withOverride(key, reverted));
    throw error;
  } finally {
    pending--;
  }
}

export function changeSpaceNotifications(spaceId: string, change: NotificationChange) {
  return changeOverride({ spaceId }, change, () => setSpaceNotifications(spaceId, change));
}

export function changeChannelNotifications(spaceId: string, channelId: string, change: NotificationChange) {
  return changeOverride({ spaceId, channelId }, change, () => setChannelNotifications(spaceId, channelId, change));
}

/** A DM is either on (level null) or off (`nothing`), plus mute. */
export function changeDirectNotifications(
  conversationId: string,
  change: { level?: "nothing" | null; mutedUntil?: MutedUntil },
) {
  return changeOverride({ conversationId }, change, () => setDirectNotifications(conversationId, change));
}

export function useNotificationSettings() {
  return useSyncExternalStore(
    subscribe,
    () => state,
    () => INITIAL,
  );
}
