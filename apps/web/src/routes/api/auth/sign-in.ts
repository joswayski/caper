import { createFileRoute } from "@tanstack/react-router";
import { getSignInUrl } from "@workos/authkit-tanstack-react-start";

export const Route = createFileRoute("/api/auth/sign-in")({
  server: { handlers: {
    GET: async () => new Response(null, { status: 302, headers: {
      location: await getSignInUrl({ data: { returnPathname: "/live" } }),
      "cache-control": "no-store",
    } }),
  } },
});
