import { createFileRoute } from "@tanstack/react-router";
import Home from "../pages/Home";

export const Route = createFileRoute("/")({
  component: Home,
  head: () => ({
    links: [
      {
        rel: "preload",
        as: "image",
        href: "/images/chat-preview-wide.webp",
        type: "image/webp",
        media: "(min-width: 1001px)",
      },
      {
        rel: "preload",
        as: "image",
        href: "/images/chat-preview-stacked.webp",
        type: "image/webp",
        media: "(max-width: 1000px)",
      },
    ],
  }),
});
