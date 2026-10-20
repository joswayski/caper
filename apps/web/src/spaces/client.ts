import type { PersonPicture, SpaceIcon } from "../account/pictures.ts";
import { observeProfiles, profileRevision } from "../account/profiles.ts";
import { unreadChanged } from "../account/app-badge";
import { directMessageErrors } from "./direct-errors.ts";
import { hasHiddenNameCharacters } from "../names.ts";

/** `iconAsset`/`iconUrl`: the owner's custom icon; absent, the space shows its initial. */
export interface Space extends SpaceIcon {
  id: string;
  name: string;
  ownerId: string;
  inviter?: { username: string; displayName: string };
  demo?: boolean;
}

export interface Channel {
  id: string;
  spaceId: string;
  name: string;
  private: boolean;
  joined?: boolean;
}

export interface ChannelInvitation {
  channel: Channel;
  inviter: { username: string; displayName: string };
}

export interface Member extends PersonPicture {
  id: string;
  avatarId?: number | null;
  username: string;
  displayName: string;
  owner: boolean;
  admin?: boolean;
}

export interface DirectConversation {
  id: string;
  /** `team`: the Caper account, whose DM goes to the feedback team. */
  peer: PersonPicture & {
    id: string;
    username: string;
    displayName: string;
    avatarId?: number;
    team?: boolean;
    admin?: boolean;
  };
  lastSeq: string;
  readSeq: string;
  /** Incoming main-timeline messages, not stream events. Older servers omit it. */
  unreadCount?: number;
  /** Older servers omit it: treat as accepted. */
  status?: "accepted" | "outgoing" | "incoming";
  /** You blocked the peer. The other side is never told. */
  blocked?: boolean;
}

/** `incoming` is a message request for you; `outgoing` waits for the other person. */
export function directStatus(conversation: DirectConversation) {
  return conversation.status ?? "accepted";
}

/** Requests for you never count as unread. */
export function directUnread(conversation: DirectConversation) {
  return (
    directStatus(conversation) !== "incoming" &&
    !conversation.blocked &&
    BigInt(conversation.lastSeq) > BigInt(conversation.readSeq) &&
    (conversation.unreadCount === undefined || conversation.unreadCount > 0)
  );
}

/** People sharing a space or DM with you, for `@` suggestions in DMs. */
export interface Person extends PersonPicture {
  id: string;
  username: string;
  displayName: string;
  avatarId?: number | null;
  admin?: boolean;
}

/** People in a response refresh live profile updates they postdate: there is no
 * replay after a gateway reconnect, so refetched lists are the repair. */
function withPeople<T>(response: Promise<T>, people: (value: T) => Iterable<Person | Member>): Promise<T> {
  const since = profileRevision();
  return response.then((value) => {
    observeProfiles(people(value), since);
    return value;
  });
}

export function listPeople() {
  return withPeople(request<{ people: Person[] }>("/api/people"), (value) => value.people);
}

export function listDirectConversations() {
  return withPeople(request<{ conversations: DirectConversation[] }>("/api/dms"), (value) =>
    value.conversations.map((conversation) => conversation.peer),
  );
}

export function lookupDirectPerson(username: string) {
  return withPeople(
    request<Person>("/api/dms/lookup", {
      method: "POST",
      body: JSON.stringify({ username: username.trim() }),
    }),
    (person) => [person],
  );
}

export function createDirectConversation(username: string) {
  return request<DirectConversation>("/api/dms", {
    method: "POST",
    body: JSON.stringify({ username: username.trim() }),
  });
}

export async function readDirectConversation(id: string, seq: string) {
  await request<void>(`/api/dms/${pathId(id)}/read`, { method: "POST", body: JSON.stringify({ seq }) });
  unreadChanged();
}

/** One `@feedback`, for the team's inbox. Replies belong in `threadRootId` (the message itself when top-level). */
export interface FeedbackReport {
  id: string;
  messageId: string;
  threadRootId: string;
  createdAt: string;
  text: string;
  attachments: number;
  reporter: Person;
  channel: { id: string; name: string; direct: boolean };
  space: { id: string; name: string } | null;
  replyCount: number;
  /** The newest message in its thread is the team's. */
  answered: boolean;
  archived?: boolean;
}

export function listFeedback(before?: string, archived = false) {
  const query = new URLSearchParams();
  if (before) query.set("before", before);
  if (archived) query.set("archived", "true");
  return request<FeedbackPage>(`/api/feedback${query.size ? `?${query}` : ""}`);
}

export interface FeedbackPage {
  reports: FeedbackReport[];
  cursor: string | null;
}

/**
 * A background refresh of the newest page. Older pages the team already loaded
 * with "Show more" stay below it instead of vanishing on every poll.
 */
export function refreshedFeedback(current: FeedbackPage | undefined, page: FeedbackPage): FeedbackPage {
  if (!current || !page.cursor) return page;
  const last = page.reports.at(-1);
  const index = last ? current.reports.findIndex((report) => report.id === last.id) : -1;
  if (index < 0) return page;
  const ids = new Set(page.reports.map((report) => report.id));
  const older = current.reports.slice(index + 1).filter((report) => !ids.has(report.id));
  return older.length ? { reports: [...page.reports, ...older], cursor: current.cursor } : page;
}

/** Hides a reported thread from the whole team's inbox, or brings it back. */
export function archiveFeedback(channelId: string, threadRootId: string, archived: boolean) {
  return request<void>("/api/feedback/archive", {
    method: "PUT",
    body: JSON.stringify({ channelId, threadRootId, archived }),
  });
}

export interface UnreadSpace {
  id: string;
  channels: { id: string; lastSeq: string; readSeq?: string }[];
}

export function listUnreadSpaces() {
  return request<{ spaces: UnreadSpace[]; feedbackWaiting?: number; badgeCount?: number }>("/api/spaces/unread");
}

export async function readChannel(spaceId: string, channelId: string, seq: string) {
  await request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/read`, {
    method: "POST",
    body: JSON.stringify({ seq }),
  });
  unreadChanged();
}

export function acceptDirectRequest(id: string) {
  return request<DirectConversation>(`/api/dms/${pathId(id)}/accept`, { method: "POST" });
}

export function declineDirectRequest(id: string) {
  return request<void>(`/api/dms/${pathId(id)}/decline`, { method: "POST" });
}

export interface BlockedAccount extends PersonPicture {
  id: string;
  username: string;
  displayName: string;
  avatarId?: number;
}

export function listBlocks() {
  return request<{ blocks: BlockedAccount[] }>("/api/blocks");
}

export function blockAccount(id: string) {
  return request<void>(`/api/blocks/${pathId(id)}`, { method: "PUT" });
}

export function unblockAccount(id: string) {
  return request<void>(`/api/blocks/${pathId(id)}`, { method: "DELETE" });
}

/** Who can start a DM with you: anyone (as a request), people in your spaces, or no one new. */
export type DirectPrivacy = "anyone" | "spaces" | "nobody";

export function getDirectPrivacy() {
  return request<{ directMessages: DirectPrivacy }>("/api/account/privacy");
}

export function setDirectPrivacy(directMessages: DirectPrivacy) {
  return request<{ directMessages: DirectPrivacy }>("/api/account/privacy", {
    method: "PUT",
    body: JSON.stringify({ directMessages }),
  });
}

/** `mentions` reads as "Only @mentions and DMs" at the account level. */
export type NotificationLevel = "all" | "mentions" | "nothing";
/** When phone push is sent; phones only, web leaves it alone. */
export type MobileNotifications = "whenInactive" | "always";
/** An RFC 3339 UTC timestamp, `forever`, or null (not muted). */
export type MutedUntil = string | null;

/** A space (`spaceId`), channel (`spaceId` + `channelId`) or DM (`conversationId`) override. */
export interface NotificationOverride {
  spaceId?: string;
  channelId?: string;
  conversationId?: string;
  /** null inherits; a DM only uses `nothing` (notifications off). */
  level: NotificationLevel | null;
  mutedUntil: MutedUntil;
  voiceStarted?: boolean | null;
}

export interface NotificationSettings {
  level: NotificationLevel;
  mobile: MobileNotifications;
  voiceStarted: boolean;
  overrides: NotificationOverride[];
}

/** A missing key leaves that field unchanged; null resets it. */
export interface NotificationChange {
  level?: NotificationLevel | null;
  mutedUntil?: MutedUntil;
  voiceStarted?: boolean | null;
}

// Clients read an unknown future level as `mentions`.
function notificationLevel(value: unknown): NotificationLevel {
  return value === "all" || value === "nothing" ? value : "mentions";
}

function notificationOverride(value: NotificationOverride): NotificationOverride {
  return {
    ...value,
    level: value.level == null ? null : notificationLevel(value.level),
    mutedUntil: value.mutedUntil ?? null,
  };
}

function notificationSettings(value: NotificationSettings): NotificationSettings {
  return {
    level: notificationLevel(value.level),
    voiceStarted: value.voiceStarted !== false,
    mobile: value.mobile === "always" ? "always" : "whenInactive",
    overrides: (value.overrides ?? []).map(notificationOverride),
  };
}

export async function getNotificationSettings() {
  return notificationSettings(await request<NotificationSettings>("/api/notifications/settings"));
}

export async function updateNotificationSettings(change: {
  level?: NotificationLevel;
  mobile?: MobileNotifications;
  voiceStarted?: boolean;
}) {
  return notificationSettings(
    await request<NotificationSettings>("/api/notifications/settings", { method: "PUT", body: JSON.stringify(change) }),
  );
}

export async function setSpaceNotifications(spaceId: string, change: NotificationChange) {
  return notificationOverride(
    await request<NotificationOverride>(`/api/spaces/${pathId(spaceId)}/notifications`, {
      method: "PUT",
      body: JSON.stringify(change),
    }),
  );
}

export async function setChannelNotifications(spaceId: string, channelId: string, change: NotificationChange) {
  return notificationOverride(
    await request<NotificationOverride>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/notifications`, {
      method: "PUT",
      body: JSON.stringify(change),
    }),
  );
}

export async function setDirectNotifications(
  conversationId: string,
  change: { level?: "nothing" | null; mutedUntil?: MutedUntil },
) {
  return notificationOverride(
    await request<NotificationOverride>(`/api/dms/${pathId(conversationId)}/notifications`, {
      method: "PUT",
      body: JSON.stringify(change),
    }),
  );
}

export interface SpaceLimits {
  ownedSpaces: number;
  totalSpaces: number;
  channelsPerSpace: number;
}

export interface SpaceDetail {
  space: Space;
  channels: Channel[];
  members: Member[];
  /** Private channels you can read: the ids of everyone who can read them (owner and grants). */
  channelMembers?: Record<string, string[]>;
  channelInvitations?: ChannelInvitation[];
}

export class SpacesApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

const SPACE_ID = /^[A-Za-z0-9]{12}$/;
const CHANNEL_NAME = /^[a-z]+(?:-[a-z]+)*$/;
// oxlint-disable-next-line no-control-regex -- Reject control characters in user-provided names.
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/u;

export function spaceNameError(name: string) {
  const trimmed = name.trim();
  if (!trimmed) return "Enter a space name.";
  if (Array.from(trimmed).length > 80) return "Space names can be at most 80 characters.";
  if (CONTROL.test(trimmed)) return "Space names cannot contain control characters.";
  if (hasHiddenNameCharacters(trimmed)) return "Space names cannot contain invisible or text-direction characters.";
}

export function channelNameError(name: string) {
  if (!name) return "Enter a channel name.";
  if (Array.from(name).length > 80) return "Channel names can be at most 80 characters.";
  if (!CHANNEL_NAME.test(name)) return "Use lowercase letters separated by single dashes.";
}

export function normalizeChannelName(value: string) {
  // Keep a trailing dash while typing so multi-word names remain editable.
  return value
    .toLowerCase()
    .replace(/\s+/g, "-")
    .replace(/[^a-z-]/g, "")
    .replace(/-+/g, "-")
    .replace(/^-/, "")
    .slice(0, 80);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    cache: init?.method ? undefined : "no-store",
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: unknown; code?: unknown } | null;
    const known = typeof body?.code === "string" ? directMessageErrors[body.code] : undefined;
    throw new SpacesApiError(
      response.status,
      known ?? (typeof body?.error === "string" ? body.error : "That request did not work."),
    );
  }
  return response.status === 204 ? (undefined as T) : (response.json() as Promise<T>);
}

function pathId(id: string) {
  if (!SPACE_ID.test(id)) throw new Error("Invalid space or channel ID.");
  return encodeURIComponent(id);
}

export function listSpaces() {
  return request<{ spaces: Space[]; invitations?: Space[]; limits: SpaceLimits }>("/api/spaces");
}

export function listSpaceVoiceActivity() {
  return request<{ spaceIds: string[] }>("/api/spaces/voice-activity", { signal: AbortSignal.timeout(10_000) });
}

export function getSpace(spaceId: string) {
  return withPeople(
    request<SpaceDetail>(`/api/spaces/${pathId(spaceId)}`, { signal: AbortSignal.timeout(10_000) }),
    (value) => value.members,
  );
}

export function createSpace(name: string) {
  return request<Space>("/api/spaces", { method: "POST", body: JSON.stringify({ name: name.trim() }) });
}

export function updateSpace(spaceId: string, name: string) {
  return request<Space>(`/api/spaces/${pathId(spaceId)}`, {
    method: "PATCH",
    body: JSON.stringify({ name: name.trim() }),
  });
}

/** Back to the space's initial (owner only). */
export function removeSpaceIcon(spaceId: string) {
  return request<Space>(`/api/spaces/${pathId(spaceId)}/icon`, { method: "DELETE" });
}

export function deleteSpace(spaceId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}`, { method: "DELETE" });
}

export function createChannel(spaceId: string, name: string, privateChannel: boolean) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels`, {
    method: "POST",
    body: JSON.stringify({ name, private: privateChannel }),
  });
}

export function updateChannel(spaceId: string, channelId: string, name: string, privateChannel: boolean) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}`, {
    method: "PATCH",
    body: JSON.stringify({ name, private: privateChannel }),
  });
}

export function deleteChannel(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}`, { method: "DELETE" });
}

export function listSpaceMembers(spaceId: string) {
  return withPeople(request<{ members: Member[] }>(`/api/spaces/${pathId(spaceId)}/members`), (value) => value.members);
}

export function addSpaceMember(spaceId: string, username: string) {
  return request<Member>(`/api/spaces/${pathId(spaceId)}/members`, {
    method: "POST",
    body: JSON.stringify({ username }),
  });
}

export function removeSpaceMember(spaceId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/members/${pathId(memberId)}`, { method: "DELETE" });
}

export function listSpaceInvitations(spaceId: string) {
  return withPeople(
    request<{ members: Member[] }>(`/api/spaces/${pathId(spaceId)}/invitations`),
    (value) => value.members,
  );
}

export function cancelSpaceInvitation(spaceId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/invitations/${pathId(memberId)}`, { method: "DELETE" });
}

export function acceptSpaceInvitation(spaceId: string) {
  return request<Space>(`/api/spaces/${pathId(spaceId)}/invitation`, { method: "POST" });
}

export function declineSpaceInvitation(spaceId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/invitation`, { method: "DELETE" });
}

export function listChannelMembers(spaceId: string, channelId: string) {
  return withPeople(
    request<{ members: Member[]; invitations?: Member[] }>(
      `/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`,
    ),
    (value) => [...value.members, ...(value.invitations ?? [])],
  );
}

export function addChannelMember(spaceId: string, channelId: string, username: string) {
  return request<Member>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`, {
    method: "POST",
    body: JSON.stringify({ username }),
  });
}

export function removeChannelMember(spaceId: string, channelId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members/${pathId(memberId)}`, {
    method: "DELETE",
  });
}

export function joinChannel(spaceId: string, channelId: string) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/membership`, {
    method: "POST",
  });
}

export function leaveChannel(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/membership`, { method: "DELETE" });
}

export function acceptChannelInvitation(spaceId: string, channelId: string) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/invitation`, {
    method: "POST",
  });
}

export function declineChannelInvitation(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/invitation`, { method: "DELETE" });
}

export interface InvitePerson extends PersonPicture {
  username: string;
  displayName: string;
  admin?: boolean;
  avatarId?: number | null;
}

export interface InviteLink {
  code: string;
  channel: { id: string; name: string } | null;
  createdBy: InvitePerson;
  uses: number;
  maxUses: number | null;
  expiresAt: string | null;
  createdAt: string;
}

export interface InvitePreview {
  code: string;
  space: SpaceIcon & { name: string };
  channel: { name: string } | null;
  inviter: InvitePerson;
  memberCount: number;
  expiresAt: string | null;
  alreadyMember?: boolean;
}

export interface InviteLinkOptions {
  channelId: string | null;
  /** Seconds; null never expires. */
  maxAge: number | null;
  /** null allows unlimited joins. */
  maxUses: number | null;
}

export type InviteExpiryUnit = "minutes" | "hours" | "days" | "never";
export const INVITE_EXPIRY_UNITS: { label: string; value: InviteExpiryUnit; seconds: number }[] = [
  { label: "minutes", value: "minutes", seconds: 60 },
  { label: "hours", value: "hours", seconds: 3_600 },
  { label: "days", value: "days", seconds: 86_400 },
  { label: "Never", value: "never", seconds: 0 },
];
export const DEFAULT_INVITE_EXPIRY = { amount: "30", unit: "days" as InviteExpiryUnit };
export const DEFAULT_INVITE_MAX_USES = "100";
/** The API's bounds: one minute to a year, and at most 10,000 uses. */
const MAX_AGE_SECONDS = { min: 60, max: 31_536_000 };
export const MAX_INVITE_USES = 10_000;

type Parsed<T> = { value: T; error?: undefined } | { value?: undefined; error: string };

function wholeNumber(text: string) {
  const value = text.trim();
  return /^\d+$/.test(value) ? Number(value) : undefined;
}

/** "Expire after": seconds for the API, or null for Never. */
export function inviteExpirySeconds(amount: string, unit: InviteExpiryUnit): Parsed<number | null> {
  if (unit === "never") return { value: null };
  const seconds = (wholeNumber(amount) ?? 0) * INVITE_EXPIRY_UNITS.find((option) => option.value === unit)!.seconds;
  return seconds >= MAX_AGE_SECONDS.min && seconds <= MAX_AGE_SECONDS.max
    ? { value: seconds }
    : { error: "Expiry must be between 1 minute and 365 days." };
}

/** "Max uses": empty means no limit. */
export function inviteMaxUses(text: string): Parsed<number | null> {
  if (!text.trim()) return { value: null };
  const uses = wholeNumber(text);
  return uses !== undefined && uses >= 1 && uses <= MAX_INVITE_USES
    ? { value: uses }
    : { error: "Max uses must be a whole number from 1 to 10,000." };
}

/** "10,000" */
function count(value: number) {
  return value.toLocaleString("en-US");
}

const INVITE_CODE = /^[A-Za-z0-9]{10}$/;

/** The code from a pasted invite: a full or scheme-less `/invite/<code>` URL, `caper://invite/<code>`, or the bare code. */
export function inviteCode(input: string): string | undefined {
  const value = input.trim();
  if (INVITE_CODE.test(value)) return value;
  const app = /^caper:\/\/invite\/([A-Za-z0-9]{10})\/?(?:[?#].*)?$/i.exec(value);
  if (app) return app[1];
  const path = value.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/[?#].*$/, "");
  const match = /^(?:[^/\s]+)?\/invite\/([A-Za-z0-9]{10})\/?$/.exec(path);
  return match?.[1];
}

export function inviteUrl(code: string, origin = window.location.origin) {
  return `${origin}/invite/${code}`;
}

/** Opens the installed Android, iPhone, Mac, Windows or Linux app, which all register `caper://`. */
export function appInviteUrl(code: string) {
  return `caper://invite/${code}`;
}

/** "#feedback · 7 of 10 joins remaining · Expires in 6 days" */
export function inviteLinkSummary(link: InviteLink, now = Date.now()) {
  const uses =
    link.maxUses === null
      ? `${count(link.uses)} ${link.uses === 1 ? "use" : "uses"}`
      : `${count(Math.max(0, link.maxUses - link.uses))} of ${count(link.maxUses)} ${link.maxUses === 1 ? "join" : "joins"} remaining`;
  return [link.channel && `#${link.channel.name}`, uses, inviteExpiry(link.expiresAt, now)].filter(Boolean).join(" · ");
}

export function inviteExpiry(expiresAt: string | null, now = Date.now()) {
  if (!expiresAt) return "Never expires";
  if (Date.parse(expiresAt) <= now) return "Expired";
  const minutes = Math.max(1, Math.round((Date.parse(expiresAt) - now) / 60_000));
  if (minutes < 60) return `Expires in ${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `Expires in ${hours} ${hours === 1 ? "hour" : "hours"}`;
  const days = Math.round(hours / 24);
  return `Expires in ${days} ${days === 1 ? "day" : "days"}`;
}

/** Async callers reject (rather than throw) for a malformed `/invite/<anything>` URL. */
function pathCode(code: string) {
  if (!INVITE_CODE.test(code)) throw new SpacesApiError(404, "This invite link is invalid or has expired.");
  return code;
}

export function listInviteLinks(spaceId: string) {
  return request<{ links: InviteLink[] }>(`/api/spaces/${pathId(spaceId)}/invite-links`);
}

export function ensureDefaultInviteLink(spaceId: string, channelId?: string | null) {
  const query = channelId ? `?${new URLSearchParams({ channelId: pathId(channelId) })}` : "";
  return request<InviteLink>(`/api/spaces/${pathId(spaceId)}/invite-links/default${query}`, { method: "POST" });
}

export async function regenerateInviteLink(spaceId: string, code: string) {
  return request<InviteLink>(`/api/spaces/${pathId(spaceId)}/invite-links/${pathCode(code)}/regenerate`, {
    method: "POST",
  });
}

export function createInviteLink(spaceId: string, options: InviteLinkOptions) {
  return request<InviteLink>(`/api/spaces/${pathId(spaceId)}/invite-links`, {
    method: "POST",
    body: JSON.stringify(options),
  });
}

export async function revokeInviteLink(spaceId: string, code: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/invite-links/${pathCode(code)}`, { method: "DELETE" });
}

export async function previewInvite(code: string) {
  return request<InvitePreview>(`/api/invites/${pathCode(code)}`);
}

export async function joinInvite(code: string) {
  return request<{ space: Space; channelId: string | null; joined: boolean }>(`/api/invites/${pathCode(code)}/join`, {
    method: "POST",
  });
}

/** Where a joined invite opens in the spaces app. */
export function invitedSpacePath(spaceId: string, channelId: string | null) {
  const query = new URLSearchParams({ space: spaceId });
  if (channelId) query.set("channel", channelId);
  return `/spaces?${query}`;
}

/** A pending invite carried through sign-in as `?invite=<code>`. */
export function inviteFromSearch(search: string) {
  const code = new URLSearchParams(search).get("invite");
  return code && INVITE_CODE.test(code) ? code : undefined;
}

/** Where to go once signed in: back to a pending invite, after the profile if it is unfinished. */
export function signedInPath(account: { username: string | null }, search: string) {
  const invite = inviteFromSearch(search);
  if (!account.username) return invite ? `/profile?invite=${invite}` : "/profile";
  return invite ? `/invite/${invite}` : "/spaces";
}
