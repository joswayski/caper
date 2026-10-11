import { MAX_ATTACHMENTS } from "./uploads.ts";

/** The drop overlay's text. Every client produces the strings in
 * shared/messages/drop-cases.json. Change them together. */
export function dropText(channel: string, direct: boolean, attached: number): { title: string; detail: string } {
  const room = Math.max(0, MAX_ATTACHMENTS - attached);
  return {
    title: `Attach to ${direct ? "" : "#"}${channel}`,
    detail:
      room === 0
        ? `${MAX_ATTACHMENTS} files attached. Remove one to add another.`
        : room === MAX_ATTACHMENTS
          ? `Up to ${MAX_ATTACHMENTS} files. Add a message before sending.`
          : `${room} more file${room === 1 ? "" : "s"}. Add a message before sending.`,
  };
}

/** Only drags carrying files (not selected text or links) open the overlay. */
export function carriesFiles(transfer: DataTransfer | null): boolean {
  return !!transfer && Array.from(transfer.types).includes("Files");
}
