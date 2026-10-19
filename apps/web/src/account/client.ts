import { withPicture, type PersonPicture } from "./pictures.ts";
import { withProfile, type Profiles } from "./profiles.ts";

export interface Account extends PersonPicture {
  id: string;
  avatarId?: number | null;
  username: string | null;
  displayName: string | null;
  debugEnabled?: boolean;
  /** Only on your own account: a retained custom picture can be restored. */
  canRestoreAvatar?: boolean;
  /** On the Caper feedback team: sees the feedback inbox and can visit reported conversations. */
  feedbackTeam?: boolean;
}

export function normalizeUsername(value: string) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9_]/g, "")
    .slice(0, 32);
}

export function usernameError(value: string) {
  if (!/^[a-z0-9_]{3,32}$/.test(value)) return "Use 3–32 lowercase letters, numbers, or underscores.";
}

export class AccountApiError extends Error {
  readonly status: number;
  readonly attemptsRemaining?: number;

  constructor(status: number, message: string, attemptsRemaining?: number) {
    super(message);
    this.status = status;
    this.attemptsRemaining = attemptsRemaining;
  }
}

let currentAccount: Account | undefined;
let profileRevision = 0;
const accountListeners = new Set<() => void>();

function rememberAccount<T extends Account | null>(account: T): T {
  const changed = JSON.stringify(currentAccount) !== JSON.stringify(account ?? undefined);
  currentAccount = account ?? undefined;
  if (changed) for (const listener of accountListeners) listener();
  return account;
}

export function getRememberedAccount() {
  return currentAccount;
}

export function subscribeAccount(listener: () => void) {
  accountListeners.add(listener);
  return () => void accountListeners.delete(listener);
}

/** The saved account (names and picture) wins over retained or in-flight member/peer snapshots. */
export function withAccountProfile<T extends { id: string; username?: string | null; displayName?: string | null }>(
  person: T,
  account: Account | undefined,
): T {
  return person.id === account?.id && account.username && account.displayName
    ? withPicture({ ...person, username: account.username, displayName: account.displayName }, account)
    : person;
}

/** Someone's latest known names and picture: a live update for them, and the saved account for you. */
export function withLatestProfile<T extends { id: string; username?: string | null; displayName?: string | null }>(
  person: T,
  account: Account | undefined,
  profiles: Profiles,
): T {
  return withAccountProfile(withProfile(person, profiles.get(person.id)), account);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: unknown; attemptsRemaining?: number } | null;
    // Proxies and gateways answer with their own JSON (`{"error": true, ...}`); only an API message is a message.
    const message = typeof body?.error === "string" ? body.error : "Account request failed";
    throw new AccountApiError(response.status, message, body?.attemptsRemaining);
  }
  return response.status === 204 ? (undefined as T) : (response.json() as Promise<T>);
}

export async function getAccount(): Promise<Account | null> {
  const revision = profileRevision;
  try {
    const account = await request<Account>("/api/account/me");
    return revision === profileRevision ? rememberAccount(account) : (currentAccount ?? null);
  } catch (error) {
    if (error instanceof AccountApiError && error.status === 401) return rememberAccount(null);
    throw error;
  }
}

export function requestEmailCode(email: string) {
  return request<{ challengeId: string }>("/api/auth/email/request", {
    method: "POST",
    body: JSON.stringify({ email }),
  });
}

export async function verifyEmailCode(challengeId: string, code: string) {
  const result = await request<{ account: Account }>("/api/auth/email/verify", {
    method: "POST",
    body: JSON.stringify({ challengeId, code, tokenTransport: "cookie" }),
  });
  return rememberAccount(result.account);
}

export async function updateProfile(username: string, displayName: string) {
  const account = await request<Account>("/api/account/profile", {
    method: "POST",
    body: JSON.stringify({ username, displayName }),
  });
  return rememberSavedAccount(account);
}

/** A saved change (profile or picture): it updates the whole running client and
 * wins over account reads that were already in flight. */
export function rememberSavedAccount(account: Account) {
  profileRevision++;
  return rememberAccount(account);
}

/** Back to the saved mascot; `avatarId` never changes. */
export async function removeAvatar() {
  return rememberSavedAccount(await request<Account>("/api/account/avatar", { method: "DELETE" }));
}

/** Compare with the displayed picture so a stale tab cannot undo a newer change. */
export async function restoreAvatar(avatarAsset: string | null) {
  return rememberSavedAccount(
    await request<Account>("/api/account/avatar/restore", {
      method: "POST",
      body: JSON.stringify({ avatarAsset }),
    }),
  );
}

/** Browser storage key for the chat capability minted from the account session. */
export const CHAT_SESSION_KEY = "caper.chat.session";

export async function logout() {
  await request<void>("/api/auth/logout", { method: "POST" });
  rememberAccount(null);
  // Logout revokes the capability too; don't leave it or the identity behind.
  try {
    localStorage.removeItem(CHAT_SESSION_KEY);
  } catch {
    /* Storage is optional. */
  }
}
