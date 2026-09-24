import { createServerFn } from "@tanstack/react-start";
import { getRequest } from "@tanstack/react-start/server";
import type { GeneralChatHistory } from "../chat/types";

/** The homepage window starts from the newest messages; the room pages the rest. */
const HOMEPAGE_MESSAGES = 16;

/** Newest public #general messages for the homepage room, or null if unavailable. */
export const getPublicChannel = createServerFn({ method: "GET" }).handler(async (): Promise<GeneralChatHistory | null> => {
  try {
    const request = getRequest();
    const response = await fetch(new URL("/api/chat/general", request.url), {
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(5_000)]),
    });
    if (!response.ok) return null;
    const history = await response.json() as GeneralChatHistory;
    if (typeof history.space?.id !== "string" || typeof history.channel?.id !== "string") return null;
    const messages = Array.isArray(history.messages) ? history.messages : [];
    return {
      ...history,
      messages: messages.slice(-HOMEPAGE_MESSAGES),
      hasMore: history.hasMore || messages.length > HOMEPAGE_MESSAGES,
    };
  } catch {
    return null;
  }
});
