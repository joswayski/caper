// Unsent composer text and staged files per conversation, kept while the page
// is open so switching channels or DMs and coming back restores both. Signing
// out reloads the page, which discards every draft.
import type { DraftAttachment } from "./Attachments.tsx";
import type { AttachmentUpload } from "./uploads.ts";

const drafts = new Map<string, string>();

export const readDraft = (key: string) => drafts.get(key) ?? "";

export function saveDraft(key: string, text: string) {
  if (text) drafts.set(key, text);
  else drafts.delete(key);
}

/** A conversation's staged files: their chips and the uploads that own the bytes. */
export interface StagedFiles {
  drafts: DraftAttachment[];
  uploads: Map<string, AttachmentUpload>;
}

const staged = new Map<string, StagedFiles>();
let visible: ((key: string, change: Partial<DraftAttachment>) => void) | undefined;

/** Keeps a conversation's files, still uploading, while another is open. */
export function stashStagedFiles(key: string, files: StagedFiles) {
  if (files.drafts.length) staged.set(key, files);
  else staged.delete(key);
}

export function takeStagedFiles(key: string): StagedFiles | undefined {
  const files = staged.get(key);
  staged.delete(key);
  return files;
}

/** Object URLs a stashed conversation still shows when it is reopened. */
export function stagedUrls() {
  return new Set(
    [...staged.values()].flatMap((files) =>
      files.drafts.flatMap((draft) => [draft.localUrl, draft.thumbnailUrl].filter((url) => url !== undefined)),
    ),
  );
}

/** The open conversation's chip updater, so uploads that started in an earlier
 * view of it keep reporting progress after it is reopened. */
export function showStagedFiles(update: typeof visible) {
  visible = update;
}

/** Upload progress (or what the editor can open) for a file in either the open
 * conversation or a stashed one. */
export function stagedFileChanged(key: string, change: Partial<DraftAttachment>) {
  for (const files of staged.values())
    files.drafts = files.drafts.map((draft) => (draft.key === key ? { ...draft, ...change } : draft));
  visible?.(key, change);
}

/** Unsent thread replies by root message ID, kept across conversation switches. */
export interface ThreadDraft {
  text: string;
  broadcast: boolean;
}
let threadDrafts: Record<string, ThreadDraft> = {};

export const readThreadDrafts = () => threadDrafts;

export function saveThreadDrafts(next: Record<string, ThreadDraft>) {
  threadDrafts = Object.fromEntries(Object.entries(next).filter(([, draft]) => draft.text));
}
