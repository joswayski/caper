// Live profile and space updates (`profiles` gateway subscription). Other
// people's latest names and pictures are kept by account ID and applied over
// every retained copy where it renders, the way the saved account already wins
// for your own identity. The apply functions here are pure.
import type { ChatAuthor, ChatMessage } from "../chat/types.ts";
import { pictureUrl, withPicture, withSpaceIcon, type PersonPicture, type SpaceIcon } from "./pictures.ts";

/** `profile.updated`: picture fields absent means the picture was removed. */
export interface ProfileUpdate extends PersonPicture {
  id: string;
  username: string;
  displayName: string;
}

/** `space.updated`: icon fields absent means the icon was removed. */
export interface SpaceUpdate extends SpaceIcon {
  id: string;
  name: string;
  ownerId: string;
}

export type LiveUpdate =
  | { type: "profile.updated"; person: ProfileUpdate }
  | { type: "space.updated"; space: SpaceUpdate };

const text = (value: unknown): value is string => typeof value === "string" && value.length > 0;
const asset = (value: unknown) => (text(value) && value.length <= 64 ? value : undefined);

/** A validated event, or undefined for anything unknown or malformed (ignored, never fatal). */
export function liveUpdate(value: unknown): LiveUpdate | undefined {
  if (!value || typeof value !== "object") return;
  const event = value as { type?: unknown; person?: unknown; space?: unknown };
  if (event.type === "profile.updated" && event.person && typeof event.person === "object") {
    const person = event.person as Record<string, unknown>;
    if (!text(person.id) || !text(person.username) || !text(person.displayName)) return;
    const avatarId = person.avatarId;
    if (avatarId !== undefined && avatarId !== null && !Number.isInteger(avatarId)) return;
    const avatarAsset = asset(person.avatarAsset);
    const avatarUrl = avatarAsset ? pictureUrl(person.avatarUrl) : undefined;
    return {
      type: "profile.updated",
      person: {
        id: person.id,
        username: person.username,
        displayName: person.displayName,
        ...(avatarId !== undefined ? { avatarId: avatarId as number | null } : {}),
        ...(avatarAsset ? { avatarAsset } : {}),
        ...(avatarUrl ? { avatarUrl } : {}),
      },
    };
  }
  if (event.type === "space.updated" && event.space && typeof event.space === "object") {
    const space = event.space as Record<string, unknown>;
    if (!text(space.id) || !text(space.name) || !text(space.ownerId)) return;
    const iconAsset = asset(space.iconAsset);
    const iconUrl = iconAsset ? pictureUrl(space.iconUrl) : undefined;
    return {
      type: "space.updated",
      space: {
        id: space.id,
        name: space.name,
        ownerId: space.ownerId,
        ...(iconAsset ? { iconAsset } : {}),
        ...(iconUrl ? { iconUrl } : {}),
      },
    };
  }
}

/** `person` with `update`'s names and picture when it is the same account. */
export function withProfile<T extends { id: string; username?: string | null; displayName?: string | null }>(
  person: T,
  update: ProfileUpdate | undefined,
): T {
  if (!update || update.id !== person.id) return person;
  const named =
    person.username === update.username && person.displayName === update.displayName
      ? person
      : { ...person, username: update.username, displayName: update.displayName };
  return withPicture(named, update);
}

/** A message author (which has one `name`) with the latest profile; guests are never accounts. */
export function withAuthorProfile<T extends ChatAuthor>(author: T, update: ProfileUpdate | undefined): T {
  if (!update || update.id !== author.id || author.isGuest) return author;
  return withPicture(author.name === update.displayName ? author : { ...author, name: update.displayName }, update);
}

/** Every author a message carries: its own, the reply reference's, the pin's,
 * thread participants' and (recursively) the forwarded original's. Text, content
 * and sequence fields are never touched. */
export function withMessageAuthors(message: ChatMessage, author: (value: ChatAuthor) => ChatAuthor): ChatMessage {
  // A message whose authors are all unchanged is returned as is.
  let changed = false;
  const latest = <T>(before: T, after: T) => {
    if (before !== after) changed = true;
    return after;
  };
  const own = latest(message.author, author(message.author));
  const replyAuthor = message.replyTo && latest(message.replyTo.author, author(message.replyTo.author));
  const pinAuthor = message.pin && latest(message.pin.author, author(message.pin.author));
  const participants = message.thread?.participants.map((person) => latest(person, author(person)));
  const forwarded =
    message.forward?.message && latest(message.forward.message, withMessageAuthors(message.forward.message, author));
  if (!changed) return message;
  return {
    ...message,
    author: own,
    ...(message.replyTo ? { replyTo: { ...message.replyTo, author: replyAuthor! } } : {}),
    ...(message.pin ? { pin: { ...message.pin, author: pinAuthor! } } : {}),
    ...(message.thread ? { thread: { ...message.thread, participants: participants! } } : {}),
    ...(message.forward?.message ? { forward: { ...message.forward, message: forwarded! } } : {}),
  };
}

export type Profiles = ReadonlyMap<string, ProfileUpdate>;

/** The space with `update`'s name, owner and icon (an icon the update omits is gone). */
export function withSpaceUpdate<T extends SpaceIcon & { id: string; name: string; ownerId: string }>(
  space: T,
  update: SpaceUpdate,
): T {
  if (space.id !== update.id) return space;
  return withSpaceIcon(space, {
    name: update.name,
    ownerId: update.ownerId,
    ...(update.iconAsset ? { iconAsset: update.iconAsset } : {}),
    ...(update.iconUrl ? { iconUrl: update.iconUrl } : {}),
  } as Partial<T> & SpaceIcon);
}

/** The list with that space updated; the same list (nothing to re-render) when it isn't there. */
export function withSpaceUpdates<T extends SpaceIcon & { id: string; name: string; ownerId: string }>(
  spaces: T[],
  update: SpaceUpdate,
): T[] {
  return spaces.some((space) => space.id === update.id)
    ? spaces.map((space) => withSpaceUpdate(space, update))
    : spaces;
}

// ---- Store -------------------------------------------------------------------

let profiles: Profiles = new Map();
/** The event count when each person's entry was last set by an event. */
const revisions = new Map<string, number>();
let revision = 0;
const listeners = new Set<() => void>();
const none: Profiles = new Map();

function changed(next: Map<string, ProfileUpdate>) {
  profiles = next;
  for (const listener of listeners) listener();
}

/** The latest profile per account ID received live on this page (a new Map on every change). */
export function getProfiles(): Profiles {
  return profiles;
}

/** Server renders know no live updates. */
export function getServerProfiles(): Profiles {
  return none;
}

export function subscribeProfiles(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Read before a request whose people should refresh the store (see `observeProfiles`). */
export function profileRevision() {
  return revision;
}

export function receiveProfile(update: ProfileUpdate) {
  revision++;
  revisions.set(update.id, revision);
  changed(new Map(profiles).set(update.id, update));
}

const same = (a: ProfileUpdate, b: ProfileUpdate) =>
  a.username === b.username &&
  a.displayName === b.displayName &&
  a.avatarId === b.avatarId &&
  a.avatarAsset === b.avatarAsset &&
  a.avatarUrl === b.avatarUrl;

/**
 * Fresh people from a request started at `since` (`profileRevision()`): there is
 * no replay after a reconnect, so a refetch repairs an entry whose event it
 * postdates. An event that arrived while the request was in flight still wins.
 */
export function observeProfiles(
  people: Iterable<PersonPicture & { id: string; username?: string | null; displayName?: string | null }>,
  since: number,
) {
  let next: Map<string, ProfileUpdate> | undefined;
  for (const person of people) {
    const current = (next ?? profiles).get(person.id);
    if (!current || (revisions.get(person.id) ?? 0) > since || !person.username || !person.displayName) continue;
    const fresh: ProfileUpdate = {
      id: person.id,
      username: person.username,
      displayName: person.displayName,
      ...(person.avatarId !== undefined ? { avatarId: person.avatarId } : {}),
      ...(person.avatarAsset ? { avatarAsset: person.avatarAsset } : {}),
      ...(person.avatarAsset && person.avatarUrl ? { avatarUrl: person.avatarUrl } : {}),
    };
    if (same(current, fresh)) continue;
    next ??= new Map(profiles);
    next.set(person.id, fresh);
  }
  if (next) changed(next);
}

/** For tests: forget every live update. */
export function resetProfilesForTests() {
  revisions.clear();
  revision = 0;
  changed(new Map());
}
