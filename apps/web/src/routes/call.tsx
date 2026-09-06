import { createFileRoute } from "@tanstack/react-router";
import Call from "../pages/Call";

export const Route = createFileRoute("/call")({
  component: Call,
  head: () => ({ meta: [{ title: "Public lobby — Caper" }] }),
});
