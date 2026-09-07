import { createFileRoute } from "@tanstack/react-router";
import { getAuth } from "@workos/authkit-tanstack-react-start";
import { sameOrigin } from "../../../server/request-origin";
import { proxyAccount } from "../../../server/account-proxy";

export const Route = createFileRoute("/api/account/profile")({
  server: { handlers: { POST: async ({ request }) => {
    const headers = { "cache-control": "no-store" };
    if (!sameOrigin(request)) return new Response(null, { status: 403, headers });
    const auth = await getAuth();
    if (!auth.user) return new Response(null, { status: 401, headers });
    return proxyAccount(request, auth.accessToken);
  } } },
});
