import { linkSegments } from "./links.ts";
import type { ChatMessage } from "./types.ts";

/** Link preview cards (docs/media.md "Link embeds"), version 1. */
export type EmbedBlock =
  | { type: "author"; name: string; handle?: string; url?: string; icon?: string }
  | { type: "title"; text: string; url?: string }
  | { type: "text"; text: string }
  | { type: "images"; items: EmbedImage[]; thumbnail?: boolean }
  | {
      type: "video";
      url: string;
      poster?: string;
      width?: number;
      height?: number;
      durationMs?: number;
      gif?: boolean;
    }
  | {
      type: "player";
      provider: EmbedPlayerProvider;
      id: string;
      /** The player page native apps open; web loads the provider directly. */
      play?: string;
      poster?: string;
      width?: number;
      height?: number;
      start?: number;
    }
  | { type: "footer"; text?: string; timestamp?: string };

export interface EmbedImage {
  url: string;
  width?: number;
  height?: number;
  alt?: string;
}

export type EmbedPlayerProvider = "youtube" | "vimeo" | "twitch";

export interface ChatEmbed {
  version: 1;
  url: string;
  site: { name: string; domain: string };
  blocks: EmbedBlock[];
}

/** Unsequenced: the cards for one revision of a message, sent once they are ready. */
export interface ChatEmbedsEvent {
  type: "message.embeds";
  schemaVersion: 1;
  channelId: string;
  messageId: string;
  revision: number;
  /** Stored as sent; drawn through `embedsOf`, which skips malformed cards. */
  embeds: unknown[];
}

const text = (value: unknown): value is string => typeof value === "string" && value.length > 0;
const optionalText = (value: unknown) => value === undefined || typeof value === "string";
const link = (value: unknown): value is string => typeof value === "string" && /^https?:\/\//i.test(value);
const optionalLink = (value: unknown) => value === undefined || link(value);
const optionalSize = (value: unknown) =>
  value === undefined || (typeof value === "number" && Number.isFinite(value) && value > 0);
const PROVIDERS = new Set<string>(["youtube", "vimeo", "twitch"]);

/** A block safe to draw, or null for unknown or malformed blocks (skipped). */
function block(value: unknown): EmbedBlock | null {
  if (!value || typeof value !== "object") return null;
  const item = value as Record<string, unknown>;
  switch (item.type) {
    case "author":
      return text(item.name) && optionalText(item.handle) && optionalLink(item.url) && optionalLink(item.icon)
        ? (item as EmbedBlock)
        : null;
    case "title":
      return text(item.text) && optionalLink(item.url) ? (item as EmbedBlock) : null;
    case "text":
      return text(item.text) ? (item as EmbedBlock) : null;
    case "images": {
      if (!Array.isArray(item.items)) return null;
      const items = item.items.filter(
        (image): image is EmbedImage =>
          !!image &&
          typeof image === "object" &&
          link(image.url) &&
          optionalSize(image.width) &&
          optionalSize(image.height) &&
          optionalText(image.alt),
      );
      return items.length ? { type: "images", items: items.slice(0, 4), thumbnail: item.thumbnail === true } : null;
    }
    case "video":
      return link(item.url) &&
        optionalLink(item.poster) &&
        optionalSize(item.width) &&
        optionalSize(item.height) &&
        optionalSize(item.durationMs)
        ? (item as EmbedBlock)
        : null;
    case "player":
      return typeof item.provider === "string" &&
        PROVIDERS.has(item.provider) &&
        text(item.id) &&
        optionalLink(item.poster) &&
        optionalSize(item.width) &&
        optionalSize(item.height) &&
        optionalSize(item.start)
        ? (item as EmbedBlock)
        : null;
    case "footer":
      return (text(item.text) || text(item.timestamp)) && optionalText(item.text) && optionalText(item.timestamp)
        ? (item as EmbedBlock)
        : null;
    default:
      return null;
  }
}

export function embedOf(value: unknown): ChatEmbed | null {
  if (!value || typeof value !== "object") return null;
  const embed = value as Partial<ChatEmbed>;
  if (embed.version !== 1 || !link(embed.url) || !embed.site || !Array.isArray(embed.blocks)) return null;
  if (!text(embed.site.name) || !text(embed.site.domain)) return null;
  const blocks = embed.blocks.map(block).filter((item): item is EmbedBlock => item !== null);
  return blocks.length ? { version: 1, url: embed.url, site: embed.site, blocks } : null;
}

/** Cards safe to draw. Malformed ones are skipped rather than rejecting the
 * message, like attachments. */
export function embedsOf(message: Pick<ChatMessage, "content" | "deletedAt">): ChatEmbed[] {
  if (message.deletedAt) return [];
  const embeds = (message.content as { embeds?: unknown }).embeds;
  if (!Array.isArray(embeds)) return [];
  return embeds.map(embedOf).filter((embed): embed is ChatEmbed => embed !== null);
}

export function isChatEmbedsEvent(value: unknown): value is ChatEmbedsEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatEmbedsEvent>;
  return (
    event.type === "message.embeds" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.messageId === "string" &&
    Number.isSafeInteger(event.revision) &&
    (event.revision ?? 0) >= 1 &&
    Array.isArray(event.embeds)
  );
}

/** The message with `embeds` when they belong to its current revision. */
export function withEmbeds(message: ChatMessage, revision: number, embeds: unknown[]): ChatMessage {
  if ((message.revision ?? 1) !== revision || message.deletedAt) return message;
  const content = { ...message.content } as ChatMessage["content"] & { embeds?: unknown[] };
  if (embeds.length) content.embeds = embeds;
  else delete content.embeds;
  return { ...message, content };
}

/** The first link in a draft that will get a card (the API's rule: links
 * written `<like this>` get none), for the composer preview. */
export function previewLink(draft: string): string | undefined {
  const segments = linkSegments(draft);
  for (const [index, segment] of segments.entries()) {
    if (!segment.href) continue;
    if (segments[index - 1]?.text.endsWith("<") && segments[index + 1]?.text.startsWith(">")) continue;
    return segment.href;
  }
  return undefined;
}

/** The card a link will get, for the composer; null when it has none or
 * previews are unavailable (guests, an unconfigured server). */
export async function fetchPreview(url: string, signal: AbortSignal): Promise<ChatEmbed | null> {
  const response = await fetch("/api/embeds/preview", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ url }),
    signal,
  });
  if (!response.ok) return null;
  const body = (await response.json()) as { embed?: unknown };
  return embedOf(body.embed);
}
