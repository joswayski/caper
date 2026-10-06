import type { ChatMessage } from "./types.ts";

/** Content revisions never replace independent reaction, pin or thread revisions. */
export function mergeEditedContent(current: ChatMessage, incoming: ChatMessage): ChatMessage {
  return (incoming.revision ?? 1) > (current.revision ?? 1)
    ? { ...current, content: incoming.content, revision: incoming.revision, editedAt: incoming.editedAt, editSeq: incoming.editSeq }
    : current;
}
