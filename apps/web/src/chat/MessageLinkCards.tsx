import { createContext, useContext, useEffect, useState } from "react";
import { Link, Lock } from "lucide-react";
import { linkSegments } from "./links.ts";
import { messageLink, type MessageLink } from "./message-links.ts";
import { isChatMessage, type ChatMessage } from "./types.ts";
import { OriginalMessage } from "./Forwarding.tsx";
import { MessageEmbeds } from "./Embeds.tsx";
import { embedsOf } from "./embeds.ts";

/** Opens a linked message in place (Spaces supplies it); without it, links open as pages. */
export const MessageLinkContext = createContext<((link: MessageLink) => void) | undefined>(undefined);
export function useOpenMessageLink() {
  return useContext(MessageLinkContext);
}

export interface ResolvedMessageLink {
  spaceId: string | null;
  message: ChatMessage;
}

/** The API answers 404 alike for no access, signed out and unknown: `null` here. */
export async function resolveMessageLink(link: MessageLink, signal?: AbortSignal) {
  const response = await fetch(
    `/api/chat/message-links/${encodeURIComponent(link.channel)}/${encodeURIComponent(link.message)}`,
    {
      cache: "no-store",
      signal: AbortSignal.any([signal ?? new AbortController().signal, AbortSignal.timeout(10_000)]),
    },
  );
  if (response.status === 404 || response.status === 401 || response.status === 403) return null;
  if (!response.ok) throw new Error("Couldn’t load that message.");
  const data = (await response.json()) as ResolvedMessageLink;
  if (!data || !isChatMessage(data.message) || !(data.spaceId === null || typeof data.spaceId === "string"))
    throw new Error("Couldn’t load that message.");
  return data;
}

/** Cards refresh this often while shown, so edits, reactions and deletions follow the original. */
const REFRESH_MS = 30_000;
const cache = new Map<string, { at: number; value: Promise<ResolvedMessageLink | null> }>();
function cached(link: MessageLink) {
  const key = `${link.channel}/${link.message}`;
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < REFRESH_MS) return hit.value;
  const value = resolveMessageLink(link);
  cache.set(key, { at: Date.now(), value });
  // A failure is retried on the next refresh rather than remembered.
  value.catch(() => cache.delete(key));
  return value;
}

/** The distinct message links in a message's text, up to five like link previews. */
export function messageLinksIn(text: string) {
  const found: MessageLink[] = [];
  for (const segment of linkSegments(text)) {
    const link = segment.href ? messageLink(segment.href) : undefined;
    if (link && !found.some((item) => item.message === link.message)) found.push(link);
    if (found.length === 5) break;
  }
  return found;
}

function MessageLinkCard({ link, href }: { link: MessageLink; href: string }) {
  const open = useOpenMessageLink();
  const [state, setState] = useState<{ value?: ResolvedMessageLink | null; failed?: boolean }>({});
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      if (document.visibilityState !== "visible") return;
      cached(link).then(
        (value) => !cancelled && setState({ value }),
        () => !cancelled && setState((current) => (current.value === undefined ? { failed: true } : current)),
      );
    };
    load();
    const timer = setInterval(load, REFRESH_MS);
    document.addEventListener("visibilitychange", load);
    return () => {
      cancelled = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", load);
    };
  }, [link.channel, link.message]);
  // Loading and failures keep the plain link above; no card claims anything yet.
  if (state.value === undefined) return null;
  if (state.value === null)
    return (
      <a className="chat-message-link-locked" href={href} onClick={(event) => event.stopPropagation()}>
        <Lock size={12} aria-hidden="true" />
        Message you can’t view
      </a>
    );
  const original = state.value.message;
  return (
    <div className="chat-forward-card chat-message-link-card">
      <small className="chat-forward-label">
        <Link size={12} aria-hidden="true" />
        Linked message
      </small>
      {original.deletedAt ? (
        <p className="chat-forward-unavailable">This message was deleted.</p>
      ) : (
        <>
          <OriginalMessage message={original} />
          <MessageEmbeds embeds={embedsOf(original)} />
        </>
      )}
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          if (open) open(link);
          else window.location.assign(href);
        }}
      >
        Jump to message
      </button>
    </div>
  );
}

/** Cards for the message links in `text`, below the message like link previews. */
export function MessageLinkCards({ text }: { text: string }) {
  const links = messageLinksIn(text);
  if (!links.length) return null;
  return links.map((link) => (
    <MessageLinkCard key={link.message} link={link} href={`/m/${link.channel}/${link.message}`} />
  ));
}
