import { createFileRoute } from "@tanstack/react-router";
import { downloadRedirect } from "../server/download-redirect";

// Same as /download, at the address people know from other apps (claude.ai/mobile).
export const Route = createFileRoute("/mobile")({
  server: { handlers: { GET: ({ request }) => downloadRedirect(request) } },
});
