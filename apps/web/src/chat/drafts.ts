// Unsent composer text per conversation, and thread replies per root, kept
// in this browser's localStorage under the signed-in account, so a reload, a
// restart or a new version restores them. Logout erases the account's drafts.
// Staged files are kept only while the page is open: switching channels or
// DMs and coming back restores them, a reload does not. Drafts never go to
// the server.
import type { DraftAttachment } from "./Attachments.tsx";
import type { AttachmentUpload } from "./uploads.ts";

const DRAFT_PREFIX = "caper:draft:";
const THREAD_PREFIX = "caper:thread-drafts:";

/** Kept when storage is unavailable (private windows, a blocked or full store). */
const drafts = new Map<string, string>();

function storage() {
  try {
    return typeof localStorage === "undefined" ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

function stored(key: string) {
  try {
    return storage()?.getItem(key) ?? undefined;
  } catch {
    return undefined;
  }
}

function store(key: string, value: string | undefined) {
  try {
    if (value === undefined) storage()?.removeItem(key);
    else storage()?.setItem(key, value);
  } catch {
    /* Storage is optional; the draft stays in memory for this page. */
  }
}

/** `key` is `<account>:<conversation>`. */
export const readDraft = (key: string) => stored(DRAFT_PREFIX + key) ?? drafts.get(key) ?? "";

export function saveDraft(key: string, text: string) {
  if (text) drafts.set(key, text);
  else drafts.delete(key);
  store(DRAFT_PREFIX + key, text || undefined);
}

/** Logout: the account's drafts leave this browser, so the next account never sees them. */
export function forgetDrafts(accountId: string) {
  for (const key of drafts.keys()) if (key.startsWith(`${accountId}:`)) drafts.delete(key);
  threadDrafts.delete(accountId);
  store(THREAD_PREFIX + accountId, undefined);
  const local = storage();
  if (!local) return;
  try {
    const prefix = `${DRAFT_PREFIX}${accountId}:`;
    for (const key of Object.keys(local)) if (key.startsWith(prefix)) local.removeItem(key);
  } catch {
    /* Nothing more to remove. */
  }
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

/** Unsent thread replies by root message ID, kept like conversation drafts. */
export interface ThreadDraft {
  text: string;
  broadcast: boolean;
}
const threadDrafts = new Map<string, Record<string, ThreadDraft>>();

/** `accountId` is undefined for the signed-out simulation, which keeps nothing. */
export function readThreadDrafts(accountId: string | undefined): Record<string, ThreadDraft> {
  const key = accountId ?? "";
  const saved = accountId ? stored(THREAD_PREFIX + accountId) : undefined;
  if (saved !== undefined) {
    try {
      const parsed: unknown = JSON.parse(saved);
      if (parsed && typeof parsed === "object") {
        return Object.fromEntries(
          Object.entries(parsed as Record<string, Partial<ThreadDraft>>)
            .filter(([, draft]) => typeof draft?.text === "string" && draft.text)
            .map(([root, draft]) => [root, { text: draft.text!, broadcast: draft.broadcast === true }]),
        );
      }
    } catch {
      /* A corrupt entry means no saved replies. */
    }
  }
  return threadDrafts.get(key) ?? {};
}

export function saveThreadDrafts(accountId: string | undefined, next: Record<string, ThreadDraft>) {
  const kept = Object.fromEntries(Object.entries(next).filter(([, draft]) => draft.text));
  threadDrafts.set(accountId ?? "", kept);
  if (accountId) store(THREAD_PREFIX + accountId, Object.keys(kept).length ? JSON.stringify(kept) : undefined);
}
