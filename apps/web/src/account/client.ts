export interface Account {
  id: string;
  username: string | null;
  displayName: string | null;
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
    return await request<Account>("/api/account/me");
  } catch (error) {
    if (error instanceof AccountApiError && error.status === 401) return null;
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
  return result.account;
}

export function updateProfile(username: string, displayName: string) {
  return request<Account>("/api/account/profile", {
    method: "POST",
    body: JSON.stringify({ username, displayName }),
  });
}

export function logout() {
  return request<void>("/api/auth/logout", { method: "POST" });
}
