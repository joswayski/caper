import { directMessageErrors } from "./direct-errors.ts";

export interface Space {
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

export interface Member {
  id: string;
  avatarId?: number | null;
  username: string;
  displayName: string;
  owner: boolean;
}

export interface DirectConversation {
  id: string;
  peer: { id: string; username: string; displayName: string; avatarId?: number };
  lastSeq: string;
  readSeq: string;
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
  return directStatus(conversation) !== "incoming" && BigInt(conversation.lastSeq) > BigInt(conversation.readSeq);
}

export function listDirectConversations() {
  return request<{ conversations: DirectConversation[] }>("/api/dms");
}

export function createDirectConversation(username: string) {
  return request<DirectConversation>("/api/dms", { method: "POST", body: JSON.stringify({ username: username.trim() }) });
}

export function readDirectConversation(id: string, seq: string) {
  return request<void>(`/api/dms/${pathId(id)}/read`, { method: "POST", body: JSON.stringify({ seq }) });
}

export function acceptDirectRequest(id: string) {
  return request<DirectConversation>(`/api/dms/${pathId(id)}/accept`, { method: "POST" });
}

export function declineDirectRequest(id: string) {
  return request<void>(`/api/dms/${pathId(id)}/decline`, { method: "POST" });
}

export interface BlockedAccount { id: string; username: string; displayName: string; avatarId?: number }

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
  return request<{ directMessages: DirectPrivacy }>("/api/account/privacy", { method: "PUT", body: JSON.stringify({ directMessages }) });
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
}

export function channelNameError(name: string) {
  if (!name) return "Enter a channel name.";
  if (Array.from(name).length > 80) return "Channel names can be at most 80 characters.";
  if (!CHANNEL_NAME.test(name)) return "Use lowercase letters separated by single dashes.";
}

export function normalizeChannelName(value: string) {
  // Keep a trailing dash while typing so multi-word names remain editable.
  return value.toLowerCase().replace(/\s+/g, "-").replace(/[^a-z-]/g, "")
    .replace(/-+/g, "-").replace(/^-/, "").slice(0, 80);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    cache: init?.method ? undefined : "no-store",
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null) as { error?: unknown; code?: unknown } | null;
    const known = typeof body?.code === "string" ? directMessageErrors[body.code] : undefined;
    throw new SpacesApiError(response.status, known ?? (typeof body?.error === "string" ? body.error : "That request did not work."));
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}

function pathId(id: string) {
  if (!SPACE_ID.test(id)) throw new Error("Invalid space or channel ID.");
  return encodeURIComponent(id);
}

export function listSpaces() {
  return request<{ spaces: Space[]; invitations?: Space[]; limits: SpaceLimits }>("/api/spaces");
}

export function getSpace(spaceId: string) {
  return request<SpaceDetail>(`/api/spaces/${pathId(spaceId)}`, { signal: AbortSignal.timeout(10_000) });
}

export function createSpace(name: string) {
  return request<Space>("/api/spaces", { method: "POST", body: JSON.stringify({ name: name.trim() }) });
}

export function updateSpace(spaceId: string, name: string) {
  return request<Space>(`/api/spaces/${pathId(spaceId)}`, { method: "PATCH", body: JSON.stringify({ name: name.trim() }) });
}

export function deleteSpace(spaceId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}`, { method: "DELETE" });
}

export function createChannel(spaceId: string, name: string, privateChannel: boolean) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels`, {
    method: "POST", body: JSON.stringify({ name, private: privateChannel }),
  });
}

export function updateChannel(spaceId: string, channelId: string, name: string, privateChannel: boolean) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}`, {
    method: "PATCH", body: JSON.stringify({ name, private: privateChannel }),
  });
}

export function deleteChannel(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}`, { method: "DELETE" });
}

export function listSpaceMembers(spaceId: string) {
  return request<{ members: Member[] }>(`/api/spaces/${pathId(spaceId)}/members`);
}

export function addSpaceMember(spaceId: string, username: string) {
  return request<Member>(`/api/spaces/${pathId(spaceId)}/members`, {
    method: "POST", body: JSON.stringify({ username }),
  });
}

export function removeSpaceMember(spaceId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/members/${pathId(memberId)}`, { method: "DELETE" });
}

export function listSpaceInvitations(spaceId: string) {
  return request<{ members: Member[] }>(`/api/spaces/${pathId(spaceId)}/invitations`);
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
  return request<{ members: Member[]; invitations?: Member[] }>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`);
}

export function addChannelMember(spaceId: string, channelId: string, username: string) {
  return request<Member>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`, {
    method: "POST", body: JSON.stringify({ username }),
  });
}

export function removeChannelMember(spaceId: string, channelId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members/${pathId(memberId)}`, { method: "DELETE" });
}

export function joinChannel(spaceId: string, channelId: string) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/membership`, { method: "POST" });
}

export function leaveChannel(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/membership`, { method: "DELETE" });
}

export function acceptChannelInvitation(spaceId: string, channelId: string) {
  return request<Channel>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/invitation`, { method: "POST" });
}

export function declineChannelInvitation(spaceId: string, channelId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/invitation`, { method: "DELETE" });
}
