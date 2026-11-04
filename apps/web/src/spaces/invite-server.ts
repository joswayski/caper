import { createServerFn } from "@tanstack/react-start";
import { getRequest, setResponseHeader } from "@tanstack/react-start/server";
import type { Account } from "../account/client";
import { getInitialAccount } from "../account/server";
import { inviteExpiry, type InvitePreview } from "./client";

export type InvitePageState =
  | { status: "invalid" }
  | { status: "unavailable" }
  | { status: "ready"; preview: InvitePreview; account: Account | null; expiry: string };

export const getInitialInvite = createServerFn({ method: "GET" })
  .inputValidator((code: string) => code)
  .handler(async ({ data: code }): Promise<InvitePageState> => {
    // The preview includes membership and the action depends on the session.
    setResponseHeader("Cache-Control", "private, no-store");
    if (!/^[A-Za-z0-9]{10}$/.test(code)) return { status: "invalid" };
    const request = getRequest();
    // Never forward a session cookie to an origin supplied by a production Host header.
    const origin = process.env.CAPER_API_ORIGIN || (import.meta.env.DEV ? new URL(request.url).origin : undefined);
    if (!origin) return { status: "unavailable" };
    const cookie = request.headers.get("cookie");
    try {
      const [response, account] = await Promise.all([
        fetch(new URL(`/api/invites/${code}`, origin), {
          headers: cookie ? { cookie } : undefined,
          redirect: "error",
          signal: AbortSignal.any([request.signal, AbortSignal.timeout(3_000)]),
        }),
        getInitialAccount(),
      ]);
      if (response.status === 404) return { status: "invalid" };
      // An account lookup failure isn't a signed-out session.
      if (!response.ok || account === undefined) return { status: "unavailable" };
      const preview = (await response.json()) as InvitePreview;
      // Serialize the relative label too: hydration may cross a minute/day boundary.
      return { status: "ready", preview, account, expiry: inviteExpiry(preview.expiresAt) };
    } catch {
      return { status: "unavailable" };
    }
  });
