import { createFileRoute } from "@tanstack/react-router";
import { getWebHealth } from "../server/health";

export const Route = createFileRoute("/health")({
  server: {
    handlers: {
      GET: ({ request }) => getWebHealth(request),
    },
  },
});
