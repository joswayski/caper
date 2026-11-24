import { isChannelMessage, sequence, type ChatMessage } from "./types.ts";

/** Up in an empty composer edits the viewer's newest message in that composer's
 * conversation: the channel timeline, or one thread's root and replies. */
export function lastEditableMessage(messages: ChatMessage[], authorId: string | undefined, threadRootId?: string) {
  if (!authorId) return undefined;
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index];
    const here = threadRootId
      ? message.id === threadRootId || message.threadRootId === threadRootId
      : isChannelMessage(message);
    if (here && message.author.id === authorId && !message.deletedAt && !message.forward) return message;
  }
  return undefined;
}

type AttachmentsSnapshot = { content: Pick<ChatMessage["content"], "attachments">; attachmentsSeq?: string };

/** Takes `incoming`'s files only when its `message.attachments` revision is
 * newer, so a replayed "processing" snapshot never overwrites "ready". */
export function withNewerAttachments(current: ChatMessage, incoming: AttachmentsSnapshot): ChatMessage {
  return sequence(incoming.attachmentsSeq ?? "0") > sequence(current.attachmentsSeq ?? "0")
    ? {
        ...current,
        content: { ...current.content, attachments: incoming.content.attachments },
        attachmentsSeq: incoming.attachmentsSeq,
      }
    : current;
}

/** Content revisions never replace independent reaction, pin, thread or
 * attachment-processing revisions. */
export function mergeEditedContent(current: ChatMessage, incoming: ChatMessage): ChatMessage {
  if ((incoming.revision ?? 1) <= (current.revision ?? 1)) return current;
  const edited: ChatMessage = {
    ...current,
    content: incoming.content,
    revision: incoming.revision,
    editedAt: incoming.editedAt,
    editSeq: incoming.editSeq,
    ...(incoming.deletedAt
      ? { deletedAt: incoming.deletedAt, forward: incoming.forward, forwardSeq: incoming.forwardSeq }
      : {}),
  };
  // The edit carries the files as of its own commit; keep a newer processing result.
  if (sequence(current.attachmentsSeq ?? "0") > sequence(incoming.attachmentsSeq ?? "0")) {
    return { ...edited, content: { ...incoming.content, attachments: current.content.attachments } };
  }
  return incoming.attachmentsSeq === undefined ? edited : { ...edited, attachmentsSeq: incoming.attachmentsSeq };
}
