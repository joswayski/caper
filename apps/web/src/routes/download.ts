import { createFileRoute } from "@tanstack/react-router";
import { downloadRedirect } from "../server/download-redirect";

export const Route = createFileRoute("/download")({
  server: { handlers: { GET: ({ request }) => downloadRedirect(request) } },
});
