import { createFileRoute } from "@tanstack/react-router";
import { appleAppSiteAssociation } from "../server/app-links";

export const Route = createFileRoute("/.well-known/apple-app-site-association")({
  server: { handlers: { GET: () => appleAppSiteAssociation() } },
});
