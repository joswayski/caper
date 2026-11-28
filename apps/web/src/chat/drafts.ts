// Unsent composer text per conversation, and thread replies per root, kept
// in this browser's localStorage under the signed-in account, so a reload, a
// restart or a new version restores them. Logout erases the account's drafts.
// Staged files are kept only while the page is open: switching channels or
// DMs and coming back restores them, a reload does not. The text of each
// draft also syncs between the account's devices (see draft-sync.ts).
import type { DraftAttachment } from "./Attachments.tsx";
import {
  createDraftSync,
  type DeviceDrafts,
  type DraftRef,
  type DraftSync,
  type DraftSyncApi,
  serverDraft,
} from "./draft-sync.ts";
import type { AttachmentUpload } from "./uploads.ts";

const DRAFT_PREFIX = "caper:draft:";
const THREAD_PREFIX = "caper:thread-drafts:";
/** Drafts edited here whose latest text the server has not confirmed yet. */
const DIRTY_PREFIX = "caper:draft-dirty:";

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
  store(DIRTY_PREFIX + accountId, undefined);
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
  /** The conversation the thread is in, which syncing the reply needs. */
  channelId?: string;
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
            .map(([root, draft]) => [
              root,
              {
                text: draft.text!,
                broadcast: draft.broadcast === true,
                ...(typeof draft.channelId === "string" && draft.channelId ? { channelId: draft.channelId } : {}),
              },
            ]),
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

/** The signed-in account's draft sync, while its live updates are connected. */
let active: { accountId: string; sync: DraftSync } | undefined;

export interface AppliedDraft extends DraftRef {
  accountId: string;
  text: string;
}
const appliedListeners = new Set<(draft: AppliedDraft) => void>();

/** Open composers follow drafts typed on the account's other devices. */
export function watchServerDrafts(listener: (draft: AppliedDraft) => void) {
  appliedListeners.add(listener);
  return () => void appliedListeners.delete(listener);
}

function deviceDrafts(accountId: string): DeviceDrafts {
  const conversationKey = (channelId: string) => `${accountId}:${channelId}`;
  const dirty = () => {
    try {
      const parsed: unknown = JSON.parse(stored(DIRTY_PREFIX + accountId) ?? "[]");
      return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [];
    } catch {
      return [];
    }
  };
  return {
    read: ({ channelId, threadRootId }) =>
      threadRootId ? (readThreadDrafts(accountId)[threadRootId]?.text ?? "") : readDraft(conversationKey(channelId)),
    write({ channelId, threadRootId }, text) {
      if (!threadRootId) return saveDraft(conversationKey(channelId), text);
      const replies = readThreadDrafts(accountId);
      saveThreadDrafts(accountId, {
        ...replies,
        [threadRootId]: { text, broadcast: replies[threadRootId]?.broadcast ?? false, channelId },
      });
    },
    list() {
      const prefix = `${accountId}:`;
      const keys = new Set([...drafts.keys()].filter((key) => key.startsWith(prefix)));
      try {
        const local = storage();
        if (local)
          for (const key of Object.keys(local))
            if (key.startsWith(DRAFT_PREFIX + prefix)) keys.add(key.slice(DRAFT_PREFIX.length));
      } catch {
        /* Only the in-memory drafts, then. */
      }
      const conversations = [...keys].map((key) => ({
        channelId: key.slice(prefix.length),
        threadRootId: null,
        text: readDraft(key),
      }));
      // Replies saved before sync did not record their conversation and stay on this device.
      const replies = Object.entries(readThreadDrafts(accountId)).flatMap(([threadRootId, draft]) =>
        draft.channelId ? [{ channelId: draft.channelId, threadRootId, text: draft.text }] : [],
      );
      return [...conversations, ...replies].filter((draft) => draft.channelId && draft.text);
    },
    dirtyIds: dirty,
    setDirty(id, value) {
      const ids = new Set(dirty());
      if (value) ids.add(id);
      else ids.delete(id);
      store(DIRTY_PREFIX + accountId, ids.size ? JSON.stringify([...ids]) : undefined);
    },
  };
}

function flushOnLeave() {
  if (document.visibilityState === "hidden") active?.sync.flush();
}
function flushOnHide() {
  active?.sync.flush();
}

/** Starts syncing the account's drafts; the caller loads `GET /api/drafts`
 * into the returned sync whenever its profiles subscription is ready. */
export function connectDraftSync(accountId: string, api: DraftSyncApi) {
  active?.sync.stop();
  const sync = createDraftSync({
    api,
    device: deviceDrafts(accountId),
    onApplied: (ref, text) => {
      for (const listener of appliedListeners) listener({ ...ref, accountId, text });
    },
  });
  const current = { accountId, sync };
  active = current;
  if (typeof window !== "undefined") {
    document.addEventListener("visibilitychange", flushOnLeave);
    window.addEventListener("pagehide", flushOnHide);
  }
  return {
    sync,
    disconnect() {
      sync.stop();
      if (active !== current) return;
      active = undefined;
      if (typeof window !== "undefined") {
        document.removeEventListener("visibilitychange", flushOnLeave);
        window.removeEventListener("pagehide", flushOnHide);
      }
    },
  };
}

/** A `draft.updated` event from the account's profiles subscription. */
export function receiveServerDraft(value: unknown) {
  const draft = serverDraft(value);
  if (draft) active?.sync.received(draft);
}

/** Saves drafts still waiting for typing to stop (leaving a conversation or thread). */
export function flushDraftSync() {
  active?.sync.flush();
}

/** The composer for `key` (`<account>:<conversation>`) was edited here. */
export function editDraft(key: string, text: string) {
  saveDraft(key, text);
  if (active && key.startsWith(`${active.accountId}:`))
    active.sync.edited({ channelId: key.slice(active.accountId.length + 1), threadRootId: null });
}

/** A thread reply was edited here; `broadcast` stays on this device. */
export function editThreadDraft(accountId: string | undefined, channelId: string, rootId: string, draft: ThreadDraft) {
  if (!accountId) return;
  saveThreadDrafts(accountId, { ...readThreadDrafts(accountId), [rootId]: { ...draft, channelId } });
  if (active?.accountId === accountId) active.sync.edited({ channelId, threadRootId: rootId });
}
