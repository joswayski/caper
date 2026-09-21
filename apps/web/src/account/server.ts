import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";
import type { Account } from "./client";

export const getInitialAccount = createServerFn({ method: "GET" }).handler(async (): Promise<Account | null> => {
  const request = getRequest();
  const cookie = request.headers.get("cookie");
  const response = await fetch(new URL("/api/account/me", request.url), {
    headers: cookie ? { cookie } : undefined,
    signal: request.signal,
  }).catch(() => null);

  if (!response?.ok) return null;
  return response.json() as Promise<Account>;
});
