import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/profile")({
  beforeLoad: () => { throw redirect({ href: "/login" }); },
});
