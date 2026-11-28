// Unsent composer text per conversation, and thread replies per root, kept
// in this browser's localStorage under the signed-in account, so a reload, a
// restart or a new version restores them. Logout erases the account's drafts.
// Staged files are kept only while the page is open: switching channels or
// DMs and coming back restores them, a reload does not. The text of each
// draft, and its files once uploaded, also sync between the account's devices
// (see draft-sync.ts).
import type { DraftAttachment } from "./Attachments.tsx";
import {
  createDraftSync,
  draftId,
  filesOf,
  type DeviceDrafts,
  type DraftFiles,
  type DraftRef,
  type DraftSync,
  type DraftSyncApi,
  type LocalDraft,
  type ServerDraft,
  serverDraft,
} from "./draft-sync.ts";
import type { ChatAttachment } from "./types.ts";
import { AttachmentUpload } from "./uploads.ts";

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
  for (const key of fileRecords.keys()) if (key.startsWith(`${accountId} `)) fileRecords.delete(key);
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
  for (const [conversation, files] of staged) {
    if (!files.drafts.some((draft) => draft.key === key)) continue;
    files.drafts = files.drafts.map((draft) => (draft.key === key ? { ...draft, ...change } : draft));
    // An upload finishing in a conversation that is not open is still an edit.
    reportStagedFiles(conversation, files.drafts);
  }
  visible?.(key, change);
}

/** A file staged on another device, shown as a ready tile from its signed URLs. */
function syncedTile(file: ChatAttachment, threadRootId: string | undefined): DraftAttachment {
  return {
    key: crypto.randomUUID(),
    threadRootId,
    name: file.name,
    sourceSize: file.size,
    storedSize: file.size,
    progress: 1,
    attachment: file,
    spoiler: file.spoiler || undefined,
    ...(file.voice
      ? { voice: { durationMs: file.durationMs ?? 0, waveform: file.waveform } }
      : // Confirmed from the downloaded file before the editor opens.
        !file.animated && (file.kind === "image" || file.kind === "video")
        ? { editable: file.kind }
        : {}),
  };
}

/** Applies a server draft's files to one composer of a conversation's staged
 * files: they become ready tiles (matched to this device's own uploads by ID,
 * else synced tiles with no local original), files still preparing or
 * uploading here stay, and uploaded tiles the draft no longer lists are taken
 * out of `uploads` and returned in `dropped` for the caller to release
 * without cancelling: another device may be sending that very file. Tile
 * order is applied by `arrange`, which works on the latest tiles, since
 * progress may have changed them since `drafts` was read. */
export function applyServerFiles(
  drafts: DraftAttachment[],
  uploads: Map<string, AttachmentUpload>,
  server: ServerDraft,
) {
  const root = server.threadRootId ?? undefined;
  const mine = drafts.filter((draft) => draft.threadRootId === root);
  const local = new Map<string, string>();
  for (const draft of mine) {
    const id = uploads.get(draft.key)?.state.attachment?.id;
    if (id) local.set(id, draft.key);
  }
  const listed = new Set(server.attachments.map((file) => file.id));
  const dropped = [...local]
    .filter(([id]) => !listed.has(id))
    .map(([, key]) => ({ draft: mine.find((draft) => draft.key === key)!, upload: uploads.get(key) }));
  for (const { draft } of dropped) uploads.delete(draft.key);
  const added = new Map<string, DraftAttachment>();
  const order = server.attachments.map((file) => {
    const key = local.get(file.id);
    if (key) return { key, spoiler: !!file.spoiler };
    const tile = syncedTile(file, root);
    uploads.set(
      tile.key,
      AttachmentUpload.synced(server.channelId, file, (next) => stagedFileChanged(tile.key, next)),
    );
    added.set(tile.key, tile);
    return { key: tile.key, spoiler: !!file.spoiler };
  });
  const gone = new Set(dropped.map(({ draft }) => draft.key));
  const placed = new Set(order.map(({ key }) => key));
  const arrange = (current: DraftAttachment[]) => {
    const tiles = new Map(current.map((draft) => [draft.key, draft]));
    const ready = order.flatMap(({ key, spoiler }) => {
      const tile = tiles.get(key) ?? added.get(key);
      return tile ? [{ ...tile, spoiler: spoiler || undefined }] : [];
    });
    return [
      ...current.filter((draft) => draft.threadRootId !== root),
      ...ready,
      ...current.filter((draft) => draft.threadRootId === root && !placed.has(draft.key) && !gone.has(draft.key)),
    ];
  };
  return { dropped, arrange };
}

/** The open conversation (`<account>:<conversation>`), which applies server files to its own tiles. */
let shownFiles: { key: string; apply: (draft: ServerDraft) => void } | undefined;

export function showDraftFiles(key: string, apply: (draft: ServerDraft) => void) {
  const current = { key, apply };
  shownFiles = current;
  return () => {
    if (shownFiles === current) shownFiles = undefined;
  };
}

/** Server files for a conversation that is not open go to its stash, so opening it shows them. */
function stashServerFiles(key: string, server: ServerDraft) {
  const files = staged.get(key) ?? { drafts: [], uploads: new Map<string, AttachmentUpload>() };
  if (!files.drafts.length && !server.attachments.length) return;
  const { dropped, arrange } = applyServerFiles(files.drafts, files.uploads, server);
  for (const { draft, upload } of dropped) {
    upload?.release();
    for (const url of [draft.localUrl, draft.thumbnailUrl]) if (url) URL.revokeObjectURL(url);
  }
  stashStagedFiles(key, { drafts: arrange(files.drafts), uploads: files.uploads });
}

/** Each draft's uploaded files as last saved or applied here, by `<account> <draftId>`.
 * Missing means unknown (nothing staged or applied since the page loaded). */
const fileRecords = new Map<string, DraftFiles>();
const recordKey = (accountId: string, ref: DraftRef) => `${accountId} ${draftId(ref)}`;

const sameFiles = (a: DraftFiles | undefined, b: DraftFiles) =>
  !!a &&
  a.ids.length === b.ids.length &&
  a.ids.every((id, index) => id === b.ids[index]) &&
  a.spoilers.length === b.spoilers.length &&
  a.spoilers.every((id) => b.spoilers.includes(id));

/** A conversation's staged tiles (`key` is `<account>:<conversation>`) changed:
 * a composer whose uploaded files (their order or spoiler marks) differ from
 * the last ones saved or applied has been edited. */
export function reportStagedFiles(key: string, tiles: DraftAttachment[]) {
  const current = active;
  if (!current || !key.startsWith(`${current.accountId}:`)) return;
  const { accountId, sync } = current;
  const channelId = key.slice(accountId.length + 1);
  const composers = new Map<string | null, DraftFiles>();
  for (const tile of tiles) {
    if (!tile.attachment) continue;
    const root = tile.threadRootId ?? null;
    const files = composers.get(root) ?? { ids: [], spoilers: [] };
    files.ids.push(tile.attachment.id);
    if (tile.spoiler) files.spoilers.push(tile.attachment.id);
    composers.set(root, files);
  }
  // Composers that had files and have none now.
  const prefix = `${accountId} ${channelId}`;
  for (const record of fileRecords.keys())
    if (record === prefix || record.startsWith(`${prefix} `)) {
      const root = record.slice(prefix.length + 1) || null;
      if (!composers.has(root)) composers.set(root, { ids: [], spoilers: [] });
    }
  for (const [threadRootId, files] of composers) {
    const ref = { channelId, threadRootId };
    const before = fileRecords.get(recordKey(accountId, ref));
    if (sameFiles(before, files) || (!before && !files.ids.length)) continue;
    fileRecords.set(recordKey(accountId, ref), files);
    sync.edited(ref);
  }
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
  const text = ({ channelId, threadRootId }: DraftRef) =>
    threadRootId ? (readThreadDrafts(accountId)[threadRootId]?.text ?? "") : readDraft(conversationKey(channelId));
  return {
    read: (ref): LocalDraft => ({ text: text(ref), files: fileRecords.get(recordKey(accountId, ref)) }),
    write(server) {
      const { channelId, threadRootId } = server;
      if (text(server) !== server.text) {
        if (!threadRootId) saveDraft(conversationKey(channelId), server.text);
        else {
          const replies = readThreadDrafts(accountId);
          saveThreadDrafts(accountId, {
            ...replies,
            [threadRootId]: { text: server.text, broadcast: replies[threadRootId]?.broadcast ?? false, channelId },
          });
        }
      }
      // Recorded first, so the tiles this produces are not an edit of their own.
      fileRecords.set(recordKey(accountId, server), filesOf(server.attachments));
      const key = conversationKey(channelId);
      if (shownFiles?.key === key) shownFiles.apply(server);
      else stashServerFiles(key, server);
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
      const local = new Map(
        [...conversations, ...replies]
          .filter((draft) => draft.channelId)
          .map((draft) => [draftId(draft), { ...draft, files: fileRecords.get(recordKey(accountId, draft)) }]),
      );
      // Drafts with files and no text.
      const owner = `${accountId} `;
      for (const [record, files] of fileRecords) {
        const id = record.slice(owner.length);
        if (!record.startsWith(owner) || local.has(id)) continue;
        const [channelId, threadRootId] = id.split(" ");
        local.set(id, { channelId, threadRootId: threadRootId || null, text: "", files });
      }
      return [...local.values()].filter((draft) => draft.text || draft.files?.ids.length);
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
