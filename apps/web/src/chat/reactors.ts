import { apiError } from "./client.ts";
import { emojiCode, emojiNames } from "./emoji.ts";
import type { ChatReaction } from "./types.ts";

export interface ReactionAuthor {
  id: string;
  username: string | null;
  displayName: string | null;
  avatarId: number | null;
}

export interface ReactionList {
  messageId: string;
  reactionSeq: string;
  reactions: { emoji: string; authors: ReactionAuthor[] }[];
}

export function isReactionList(value: unknown): value is ReactionList {
  if (!value || typeof value !== "object") return false;
  const list = value as Partial<ReactionList>;
  const optionalText = (text: unknown) => text === null || typeof text === "string";
  return (
    typeof list.messageId === "string" &&
    typeof list.reactionSeq === "string" &&
    /^(0|[1-9]\d*)$/.test(list.reactionSeq) &&
    Array.isArray(list.reactions) &&
    list.reactions.every(
      (reaction) =>
        !!reaction &&
        typeof reaction.emoji === "string" &&
        Array.isArray(reaction.authors) &&
        reaction.authors.every(
          (author) =>
            !!author &&
            typeof author.id === "string" &&
            optionalText(author.username) &&
            optionalText(author.displayName) &&
            (author.avatarId === null || typeof author.avatarId === "number"),
        ),
    )
  );
}

export function reactorName(author: Pick<ReactionAuthor, "displayName" | "username">) {
  return author.displayName || author.username || "Someone";
}

/** The emoji's catalog name as `:name:`, or the glyph when it has none. */
export function emojiLabel(emoji: string, name?: string) {
  return name ? `:${name}:` : emoji;
}

/**
 * Slack/Discord-style summary shared word for word with the native clients:
 * "You, Alice, Bob and 2 others reacted with :thumbs-up:".
 */
export function reactionSummary(people: { id: string; name: string }[], selfId: string | undefined, label: string) {
  const names =
    selfId && people.some((person) => person.id === selfId)
      ? ["You", ...people.filter((person) => person.id !== selfId).map((person) => person.name)]
      : people.map((person) => person.name);
  const shown = names.slice(0, 3);
  const others = names.length - shown.length;
  const who =
    others > 0
      ? `${shown.join(", ")} and ${others} ${others === 1 ? "other" : "others"}`
      : shown.length > 1
        ? `${shown.slice(0, -1).join(", ")} and ${shown.at(-1)}`
        : (shown[0] ?? "Someone");
  return `${who} reacted with ${label}`;
}

/** Shown from the message snapshot until names load, or if they can't. */
export function fallbackSummary(reaction: ChatReaction, selfId: string | undefined, label: string) {
  const count = reaction.authorIds.length;
  if (count === 1 && reaction.authorIds[0] === selfId) return `You reacted with ${label}`;
  return `${count} ${count === 1 ? "person" : "people"} reacted with ${label}`;
}

const bareCode = (code: string) =>
  code
    .split("-")
    .filter((point) => point !== "fe0f")
    .join("-");
let names: Promise<Map<string, string>> | undefined;

/** Code points (ignoring U+FE0F) to the picker's preferred dashed name. */
export function emojiNameIndex(emojis: Record<string, { n: string[]; u: string }[]>) {
  const byCode = new Map<string, string>();
  for (const entries of Object.values(emojis)) {
    for (const entry of entries) byCode.set(bareCode(entry.u), emojiNames(entry.n).at(-1)!);
  }
  return byCode;
}

/** Catalog names from the picker's data, loaded on first use. */
export function loadEmojiNames() {
  names ??= import("emoji-picker-react/dist/data/emojis-en")
    .then(({ default: english }) => emojiNameIndex(english.emojis))
    .catch((error: unknown) => {
      names = undefined;
      throw error;
    });
  return names;
}

export function emojiNameFrom(index: Map<string, string>, emoji: string) {
  return index.get(bareCode(emojiCode(emoji)));
}

export async function emojiName(emoji: string) {
  try {
    return emojiNameFrom(await loadEmojiNames(), emoji);
  } catch {
    return undefined;
  }
}

const cache = new Map<string, ReactionList>();
const inflight = new Map<string, Promise<ReactionList>>();
const cacheKey = (channelId: string, messageId: string) => `${channelId}\n${messageId}`;

/** A cached list that is current for the message's snapshot, if any. */
export function cachedReactors(channelId: string, messageId: string, reactionSeq = "0") {
  const hit = cache.get(cacheKey(channelId, messageId));
  return hit && hit.reactionSeq === reactionSeq ? hit : undefined;
}

export function loadReactors(channelId: string, messageId: string, reactionSeq = "0"): Promise<ReactionList> {
  const key = cacheKey(channelId, messageId);
  const hit = cachedReactors(channelId, messageId, reactionSeq);
  if (hit) return Promise.resolve(hit);
  const pending = inflight.get(`${key}\n${reactionSeq}`);
  if (pending) return pending;
  const request = (async () => {
    const response = await fetch(
      `/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}/reactions`,
      {
        cache: "no-store",
        signal: AbortSignal.timeout(10_000),
      },
    );
    if (!response.ok) throw await apiError(response, "Reactions are unavailable.");
    const list: unknown = await response.json();
    if (!isReactionList(list) || list.messageId !== messageId)
      throw new Error("The chat service returned invalid reactions.");
    cache.delete(key);
    cache.set(key, list);
    if (cache.size > 200) cache.delete(cache.keys().next().value!);
    return list;
  })().finally(() => inflight.delete(`${key}\n${reactionSeq}`));
  inflight.set(`${key}\n${reactionSeq}`, request);
  return request;
}
