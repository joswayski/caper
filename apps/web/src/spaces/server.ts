import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";
import type { GeneralChatHistory } from "../chat/types";

/** The homepage window shows only the newest messages; the demo page pages the rest. */
const HOMEPAGE_MESSAGES = 16;

export interface PublicDemo {
  href: string;
  history: GeneralChatHistory | null;
}

async function loadPublicDemo(): Promise<PublicDemo> {
  try {
    const request = getRequest();
    const response = await fetch(new URL("/api/chat/general", request.url), {
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(5_000)]),
    });
    if (!response.ok) return { href: "/spaces", history: null };
    const history = await response.json() as GeneralChatHistory;
    if (typeof history.space?.id !== "string" || typeof history.channel?.id !== "string") return { href: "/spaces", history: null };
    const href = `/spaces?${new URLSearchParams({ space: history.space.id, channel: history.channel.id })}`;
    const messages = Array.isArray(history.messages) ? history.messages : [];
    return {
      href,
      history: {
        ...history,
        messages: messages.slice(-HOMEPAGE_MESSAGES),
        hasMore: history.hasMore || messages.length > HOMEPAGE_MESSAGES,
      },
    };
  } catch {
    return { href: "/spaces", history: null };
  }
}

// Resolve deployment-specific IDs so demo links point straight to the channel.
export const getPublicDemoHref = createServerFn({ method: "GET" }).handler(async () => (await loadPublicDemo()).href);

/** The demo link plus the newest public #general messages for the live homepage window. */
export const getPublicDemo = createServerFn({ method: "GET" }).handler(loadPublicDemo);
