import { createServerFn } from "@tanstack/react-start";
import { redirect } from "@tanstack/react-router";
import { getAuth } from "@workos/authkit-tanstack-react-start";

export interface Account {
  id: string;
  username: string | null;
  displayName: string | null;
}

// Only profile data crosses the server-function boundary, never refresh tokens.
export const loadAccount = createServerFn({ method: "GET" }).handler(async (): Promise<Account> => {
  const auth = await getAuth();
  if (!auth.user) throw redirect({ href: "/login" });
  const base = process.env.MEDIA_API_URL;
  if (!base) throw new Error("The account service is not configured yet.");
  const response = await fetch(`${base.replace(/\/$/, "")}/api/account/me`, {
    headers: { authorization: `Bearer ${auth.accessToken}` },
    signal: AbortSignal.timeout(10_000),
    redirect: "error",
  });
  if (response.status === 401) throw redirect({ href: "/api/auth/sign-in" });
  if (!response.ok) throw new Error("The account service is temporarily unavailable. Please try again.");
  return await response.json() as Account;
});
