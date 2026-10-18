/**
 * Consecutive messages from one person within five minutes, on the same day,
 * render as one compact block (no repeated avatar or name). Presentation only.
 * Native clients implement the same rule; see docs/media.md "Message text".
 */
import { blockedAuthor } from "./blocked.ts";
import { isChannelMessage, type ChatMessage } from "./types.ts";

export const GROUP_WINDOW_MS = 5 * 60_000;

interface Groupable {
  author?: { id: string } | null;
  createdAt: string;
  threadRootId?: string | null;
  replyTo?: object;
}

export function groupsWithPrevious(
  previous: Groupable | undefined,
  message: Groupable,
  { inThread = false }: { inThread?: boolean } = {},
) {
  if (!previous?.author || !message.author || previous.author.id !== message.author.id) return false;
  if (message.replyTo) return false;
  // A "Replied to a thread" broadcast keeps its own header above its context line.
  if (!inThread && message.threadRootId) return false;
  const before = new Date(previous.createdAt);
  const after = new Date(message.createdAt);
  const gap = after.valueOf() - before.valueOf();
  if (!Number.isFinite(gap) || Math.abs(gap) > GROUP_WINDOW_MS) return false;
  return before.toDateString() === after.toDateString();
}

/** Keep the entry boundary out of truncated history rather than labeling its first loaded row. */
export function channelNewMessagesId(
  messages: ChatMessage[],
  entry: { readSeq?: string; lastSeq: string } | undefined,
  hasMore: boolean,
  viewerId: string | undefined,
  blocked: ReadonlySet<string>,
) {
  if (entry?.readSeq === undefined || !messages.length) return undefined;
  const read = BigInt(entry.readSeq);
  if (hasMore && BigInt(messages[0].seq) > read + 1n) return undefined;
  return messages.find(
    (message) =>
      isChannelMessage(message) &&
      message.author.id !== viewerId &&
      !blockedAuthor(message.author, blocked, viewerId) &&
      BigInt(message.seq) > read &&
      BigInt(message.seq) <= BigInt(entry.lastSeq),
  )?.id;
}
