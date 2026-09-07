import { createFileRoute } from "@tanstack/react-router";
import { proxyNative } from "../../../server/native";

export const Route = createFileRoute("/api/native/$")({
  server: { handlers: {
    GET: ({ request }) => proxyNative(request),
    POST: ({ request }) => proxyNative(request),
  } },
});
