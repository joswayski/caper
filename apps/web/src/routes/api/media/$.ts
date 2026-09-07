import { createFileRoute } from "@tanstack/react-router";
import { getAuth } from "@workos/authkit-tanstack-react-start";
import { proxyMedia } from "../../../server/media";
import { sameOrigin } from "../../../server/request-origin";

async function media(request: Request) {
  if (request.method === "POST" && !sameOrigin(request)) return new Response(null, { status: 403 });
  const auth = await getAuth();
  if (!auth.user) return new Response(null, { status: 401, headers: { "cache-control": "no-store" } });
  return proxyMedia(request, auth.accessToken);
}

export const Route = createFileRoute("/api/media/$")({
  server: { handlers: {
    GET: ({ request }) => media(request),
    POST: ({ request }) => media(request),
  } },
});
