import { createFileRoute } from "@tanstack/react-router";
import { androidAssetLinks } from "../server/app-links";

export const Route = createFileRoute("/.well-known/assetlinks.json")({
  server: { handlers: { GET: () => androidAssetLinks() } },
});
