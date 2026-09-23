export interface Space {
  id: string;
  name: string;
  ownerId: string;
}

export interface Channel {
  id: string;
  spaceId: string;
  name: string;
  private: boolean;
}

export interface Member {
  id: string;
  username: string;
  displayName: string;
  owner: boolean;
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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    cache: init?.method ? undefined : "no-store",
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null) as { error?: unknown } | null;
    throw new SpacesApiError(response.status, typeof body?.error === "string" ? body.error : "That request did not work.");
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}

function pathId(id: string) {
  if (!SPACE_ID.test(id)) throw new Error("Invalid space or channel ID.");
  return encodeURIComponent(id);
}

export function listSpaces() {
  return request<{ spaces: Space[]; limits: SpaceLimits }>("/api/spaces");
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

export function listChannelMembers(spaceId: string, channelId: string) {
  return request<{ members: Member[] }>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`);
}

export function addChannelMember(spaceId: string, channelId: string, username: string) {
  return request<Member>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members`, {
    method: "POST", body: JSON.stringify({ username }),
  });
}

export function removeChannelMember(spaceId: string, channelId: string, memberId: string) {
  return request<void>(`/api/spaces/${pathId(spaceId)}/channels/${pathId(channelId)}/members/${pathId(memberId)}`, { method: "DELETE" });
}
