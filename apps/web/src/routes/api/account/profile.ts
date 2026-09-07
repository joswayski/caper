import { createFileRoute } from "@tanstack/react-router";
import { sameOrigin } from "../../../server/request-origin";

export const Route = createFileRoute("/api/account/profile")({
  server: { handlers: { POST: async ({ request }) => {
    const headers = { "cache-control": "no-store" };
    if (!sameOrigin(request)) return new Response(null, { status: 403, headers });
    return Response.json({ error: "Accounts are currently unavailable." }, { status: 503, headers });
  } } },
});
