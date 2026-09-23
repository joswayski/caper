import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";

// Resolve deployment-specific IDs so demo links point straight to the channel.
export const getPublicDemoHref = createServerFn({ method: "GET" }).handler(async () => {
  try {
    const request = getRequest();
    const response = await fetch(new URL("/api/chat/general", request.url), {
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(5_000)]),
    });
    if (!response.ok) return "/spaces";
    const history = await response.json();
    if (typeof history.space?.id !== "string" || typeof history.channel?.id !== "string") return "/spaces";
    return `/spaces?${new URLSearchParams({ space: history.space.id, channel: history.channel.id })}`;
  } catch {
    return "/spaces";
  }
});
