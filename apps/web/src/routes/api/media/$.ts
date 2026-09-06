import { createFileRoute } from "@tanstack/react-router";
import { proxyMedia } from "../../../server/media";

export const Route = createFileRoute("/api/media/$")({
  server: { handlers: {
    GET: ({ request }) => proxyMedia(request),
    POST: ({ request }) => proxyMedia(request),
  } },
});
