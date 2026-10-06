import type { ChatMessage } from "./types.ts";

/** Content revisions never replace independent reaction, pin or thread revisions. */
export function mergeEditedContent(current: ChatMessage, incoming: ChatMessage): ChatMessage {
  return (incoming.revision ?? 1) > (current.revision ?? 1)
    ? { ...current, content: incoming.content, revision: incoming.revision, editedAt: incoming.editedAt, editSeq: incoming.editSeq }
    : current;
}

/** A bounded, Unicode-safe changed-span diff. Keeps unchanged edges and shows
 * the full replacement between them, rather than pretending it is a minimal diff. */
export function textChange(before: string, after: string) {
  // Whole words keep Friday → Saturday readable rather than highlighting Fri → Satur.
  const tokens = (text: string) => text.match(/\s+|[\p{L}\p{N}_]+|[^\s\p{L}\p{N}_]+/gu) ?? [];
  const old = tokens(before), next = tokens(after);
  let start = 0;
  while (start < old.length && start < next.length && old[start] === next[start]) start++;
  let end = 0;
  while (end < old.length - start && end < next.length - start && old[old.length - end - 1] === next[next.length - end - 1]) end++;
  return {
    prefix: old.slice(0, start).join(""),
    removed: old.slice(start, old.length - end).join(""),
    added: next.slice(start, next.length - end).join(""),
    suffix: end ? old.slice(old.length - end).join("") : "",
  };
}
