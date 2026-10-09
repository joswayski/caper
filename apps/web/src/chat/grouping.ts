/**
 * Consecutive messages from one person within five minutes, on the same day,
 * render as one compact block (no repeated avatar or name). Presentation only.
 * Native clients implement the same rule; see docs/media.md "Message text".
 */
export const GROUP_WINDOW_MS = 5 * 60_000;

interface Groupable {
  author?: { id: string } | null;
  createdAt: string;
  threadRootId?: string | null;
}

export function groupsWithPrevious(
  previous: Groupable | undefined,
  message: Groupable,
  { inThread = false }: { inThread?: boolean } = {},
) {
  if (!previous?.author || !message.author || previous.author.id !== message.author.id) return false;
  // A "Replied to a thread" broadcast keeps its own header above its context line.
  if (!inThread && message.threadRootId) return false;
  const before = new Date(previous.createdAt);
  const after = new Date(message.createdAt);
  const gap = after.valueOf() - before.valueOf();
  if (!Number.isFinite(gap) || Math.abs(gap) > GROUP_WINDOW_MS) return false;
  return before.toDateString() === after.toDateString();
}
