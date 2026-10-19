import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";
import type { Account } from "./client";

const INITIAL_ACCOUNT_TIMEOUT_MS = 3_000;

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
    // A slow API must not hold the homepage render; the browser asks again itself.
    const response = await fetch(new URL("/api/account/me", origin), {
      headers: cookie ? { cookie } : undefined,
      redirect: "error",
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(INITIAL_ACCOUNT_TIMEOUT_MS)]),
    }).catch(() => null);

    // Only a refused session means signed out: an outage must not show a
    // signed-in visitor "Sign in", so leave the answer to the browser.
    if (response?.status === 401) return null;
    if (!response?.ok) return undefined;
    return (await response.json().catch(() => undefined)) as Account | undefined;
  },
);
