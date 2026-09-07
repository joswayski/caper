import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/live")({
  beforeLoad: () => { throw redirect({ href: "/login" }); },
});
