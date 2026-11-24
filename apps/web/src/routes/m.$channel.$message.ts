import { createFileRoute } from "@tanstack/react-router";

/**
 * A message link (Copy link). Spaces resolves it after sign-in, which carries
 * the query through `/login` and `/profile`, and asks the API whether the
 * reader may see the message before opening it.
 */
export const Route = createFileRoute("/m/$channel/$message")({
  server: {
    handlers: {
      GET: ({ params }) => {
        const valid = /^[A-Za-z0-9]{12}$/.test(params.channel) && /^[A-Za-z0-9]{15}$/.test(params.message);
        return new Response(null, {
          status: 302,
          headers: {
            location: valid ? `/spaces?link=${params.channel}/${params.message}` : "/spaces",
            "cache-control": "private, no-store",
          },
        });
      },
    },
  },
});
