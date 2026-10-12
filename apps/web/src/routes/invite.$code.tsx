import { createFileRoute } from "@tanstack/react-router";
import InvitePage from "../spaces/InvitePage";

export const Route = createFileRoute("/invite/$code")({
  head: () => ({
    meta: [
      { title: "You're invited - Caper" },
      { property: "og:title", content: "You're invited to a space on Caper" },
      { property: "og:description", content: "Open the link to join the conversation." },
      { name: "robots", content: "noindex" },
    ],
  }),
  component: Invite,
});

function Invite() {
  return <InvitePage code={Route.useParams().code} />;
}
