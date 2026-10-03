export interface Account {
  id: string;
  avatarId?: number | null;
  username: string | null;
  displayName: string | null;
  debugEnabled?: boolean;
}

export function normalizeUsername(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9_]/g, "").slice(0, 32);
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

function rememberAccount<T extends Account | null>(account: T): T {
  currentAccount = account ?? undefined;
  return account;
}

export function getRememberedAccount() {
  return currentAccount;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: init?.body ? { "content-type": "application/json", ...init.headers } : init?.headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null) as { error?: string; attemptsRemaining?: number } | null;
    throw new AccountApiError(response.status, body?.error ?? "Account request failed", body?.attemptsRemaining);
  }
  return response.status === 204 ? undefined as T : response.json() as Promise<T>;
}

export async function getAccount(): Promise<Account | null> {
  try {
    return rememberAccount(await request<Account>("/api/account/me"));
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
  return rememberAccount(await request<Account>("/api/account/profile", {
    method: "POST",
    body: JSON.stringify({ username, displayName }),
  }));
}

export async function logout() {
  await request<void>("/api/auth/logout", { method: "POST" });
  rememberAccount(null);
}
