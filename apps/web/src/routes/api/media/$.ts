import { createFileRoute } from "@tanstack/react-router";
import { sameOrigin } from "../../../server/request-origin";

async function media(request: Request) {
  if (request.method === "POST" && !sameOrigin(request)) return new Response(null, { status: 403 });
  return Response.json({ error: "Calls are currently unavailable." }, {
    status: 503, headers: { "cache-control": "no-store" },
  });
}

export const Route = createFileRoute("/api/media/$")({
  server: { handlers: {
    GET: ({ request }) => media(request),
    POST: ({ request }) => media(request),
  } },
});
