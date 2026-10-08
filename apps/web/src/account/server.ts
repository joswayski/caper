import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";
import type { Account } from "./client";

// The session cookie only goes to a configured API origin. Deriving it from the
// request would follow a client-controlled Host header, and use plain HTTP when
// TLS ends before this server. Returns undefined when there is nowhere trusted
// to ask, so the browser looks the account up itself.
export const getInitialAccount = createServerFn({ method: "GET" }).handler(
  async (): Promise<Account | null | undefined> => {
    const request = getRequest();
    const origin = process.env.CAPER_API_ORIGIN || (import.meta.env.DEV ? new URL(request.url).origin : undefined);
    if (!origin) return undefined;
    const cookie = request.headers.get("cookie");
    const response = await fetch(new URL("/api/account/me", origin), {
      headers: cookie ? { cookie } : undefined,
      redirect: "error",
      signal: request.signal,
    }).catch(() => null);

    if (!response?.ok) return null;
    return response.json() as Promise<Account>;
  },
);
