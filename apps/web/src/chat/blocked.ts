/** A run of consecutive messages from accounts you blocked, shown as one row. */
export interface BlockedRun { first: string; count: number }

/**
 * Maps each hidden message's key to its run. Guests and your own messages
 * never collapse, and a run is keyed by its first message so "Show" survives
 * new messages arriving after it.
 */
export function blockedRuns<T extends { clientMessageId: string; author?: { id: string; isGuest?: boolean } }>(
  messages: readonly T[], blocked: ReadonlySet<string>, ownId: string | undefined,
) {
  const runs = new Map<string, BlockedRun>();
  if (!blocked.size) return runs;
  const hidden = (message: T) => !!message.author && !message.author.isGuest && message.author.id !== ownId && blocked.has(message.author.id);
  for (let start = 0; start < messages.length;) {
    if (!hidden(messages[start])) { start++; continue; }
    let end = start;
    while (end < messages.length && hidden(messages[end])) end++;
    const run = { first: messages[start].clientMessageId, count: end - start };
    for (let index = start; index < end; index++) runs.set(messages[index].clientMessageId, run);
    start = end;
  }
  return runs;
}

export function blockedLabel(count: number) {
  return `${count} blocked ${count === 1 ? "message" : "messages"}`;
}
