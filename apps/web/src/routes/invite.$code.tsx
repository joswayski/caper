import { createFileRoute } from "@tanstack/react-router";
import { pictureUrl } from "../account/pictures";
import InvitePage from "../spaces/InvitePage";
import { getInitialInvite } from "../spaces/invite-server";

export const Route = createFileRoute("/invite/$code")({
  loader: ({ params }) => getInitialInvite({ data: params.code }),
  headers: () => ({ "Cache-Control": "private, no-store" }),
  head: ({ loaderData }) => ({
    // Start downloading the server-rendered pictures with the page instead of after layout.
    links:
      loaderData?.status === "ready"
        ? [loaderData.preview.space.iconUrl, loaderData.preview.inviter.avatarUrl]
            .map(pictureUrl)
            .filter((href) => href !== undefined)
            .map((href) => ({ rel: "preload", as: "image", href }))
        : [],
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
  const { code } = Route.useParams();
  const state = Route.useLoaderData();
  return <InvitePage key={code} code={code} state={state} />;
}
