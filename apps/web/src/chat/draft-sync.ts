// Syncs composer drafts between the account's devices (docs/media.md, "Draft
// sync"). The device copy stays the instant source; this decides when it goes
// to the server and when a server draft replaces it:
// 1. A local edit marks the draft dirty and saves it a second after typing
//    stops (at once when it is cleared, as a send does, or on flush). One save
//    per draft at a time; a failed save stays dirty until the next edit or GET.
// 2. A server draft applies unless the draft is dirty or it is not newer than
//    the last `updatedAt` applied for it. A save's response counts as applied.
// 3. After GET, a non-empty device draft with no server entry is saved.

export interface DraftRef {
  channelId: string;
  /** The thread root's message ID, or null for the conversation's composer. */
  threadRootId: string | null;
}

export interface ServerDraft extends DraftRef {
  text: string;
  /** Fixed-width UTC with microseconds, so two compare as strings. */
  updatedAt: string;
}

export interface DraftSyncApi {
  save(draft: DraftRef & { text: string }): Promise<ServerDraft>;
}

/** This device's copy of the account's drafts. */
export interface DeviceDrafts {
  read(ref: DraftRef): string;
  /** Applies a server draft to the device copy (thread drafts keep `broadcast`). */
  write(ref: DraftRef, text: string): void;
  /** Every non-empty draft this device can name a conversation for. */
  list(): (DraftRef & { text: string })[];
  /** Dirty drafts survive a reload, so offline edits are not lost to an older server copy. */
  dirtyIds(): string[];
  setDirty(id: string, dirty: boolean): void;
}

interface Entry {
  ref: DraftRef;
  dirty: boolean;
  applied?: string;
  timer?: ReturnType<typeof setTimeout>;
  saving: boolean;
  /** Another save was asked for while one was in flight. */
  again: boolean;
}

export const SAVE_DELAY_MS = 1000;

export const draftId = ({ channelId, threadRootId }: DraftRef) =>
  threadRootId ? `${channelId} ${threadRootId}` : channelId;

function refFor(id: string): DraftRef {
  const [channelId, threadRootId] = id.split(" ");
  return { channelId, threadRootId: threadRootId || null };
}

/** A well-formed draft from `GET /api/drafts`, a save or `draft.updated`, else undefined. */
export function serverDraft(value: unknown): ServerDraft | undefined {
  const draft = value as Partial<ServerDraft> | null;
  if (
    !draft ||
    typeof draft !== "object" ||
    typeof draft.channelId !== "string" ||
    !draft.channelId ||
    (draft.threadRootId != null && (typeof draft.threadRootId !== "string" || !draft.threadRootId)) ||
    typeof draft.text !== "string" ||
    typeof draft.updatedAt !== "string"
  )
    return undefined;
  return {
    channelId: draft.channelId,
    threadRootId: draft.threadRootId ?? null,
    text: draft.text,
    updatedAt: draft.updatedAt,
  };
}

export function createDraftSync({
  api,
  device,
  onApplied,
  delayMs = SAVE_DELAY_MS,
}: {
  api: DraftSyncApi;
  device: DeviceDrafts;
  /** A server draft applied (the device copy now holds it); an open composer shows it. */
  onApplied: (ref: DraftRef, text: string) => void;
  delayMs?: number;
}) {
  const entries = new Map<string, Entry>();
  let stopped = false;
  const entry = (ref: DraftRef) => {
    const id = draftId(ref);
    let found = entries.get(id);
    if (!found) {
      found = {
        ref: { channelId: ref.channelId, threadRootId: ref.threadRootId },
        dirty: false,
        saving: false,
        again: false,
      };
      entries.set(id, found);
    }
    return found;
  };
  const markDirty = (item: Entry, dirty: boolean) => {
    if (item.dirty === dirty) return;
    item.dirty = dirty;
    device.setDirty(draftId(item.ref), dirty);
  };
  for (const id of device.dirtyIds()) entry(refFor(id)).dirty = true;

  async function save(item: Entry) {
    clearTimeout(item.timer);
    item.timer = undefined;
    if (stopped || !item.dirty) return;
    if (item.saving) {
      item.again = true;
      return;
    }
    item.saving = true;
    item.again = false;
    const text = device.read(item.ref);
    try {
      const saved = await api.save({ ...item.ref, text });
      if (stopped) return;
      if (item.applied === undefined || saved.updatedAt > item.applied) item.applied = saved.updatedAt;
      // Edits made while it was in flight keep the draft dirty for their own save.
      if (device.read(item.ref) === text) markDirty(item, false);
    } catch {
      // Stays dirty; only a newer edit asks again before the next GET.
      if (device.read(item.ref) === text) item.again = false;
    } finally {
      item.saving = false;
    }
    if (item.again && !stopped) void save(item);
  }

  function receive(value: ServerDraft) {
    if (stopped) return;
    const item = entry(value);
    if (item.dirty || (item.applied !== undefined && value.updatedAt <= item.applied)) return;
    item.applied = value.updatedAt;
    // Another tab of this account may have written the device copy already;
    // this tab's open composer still has to show it.
    if (device.read(value) !== value.text) device.write(value, value.text);
    onApplied(item.ref, value.text);
  }

  return {
    /** The device copy of `ref` was just edited here. */
    edited(ref: DraftRef) {
      if (stopped) return;
      const item = entry(ref);
      markDirty(item, true);
      clearTimeout(item.timer);
      // Clearing (a send, or deleting everything) saves at once.
      if (!device.read(ref)) void save(item);
      else item.timer = setTimeout(() => void save(item), delayMs);
    },
    /** Saves every draft still waiting out its delay (leaving, backgrounding). */
    flush() {
      for (const item of entries.values()) if (item.timer !== undefined) void save(item);
    },
    /** A `draft.updated` event. */
    received: receive,
    /** The drafts from `GET /api/drafts`. */
    loaded(drafts: ServerDraft[]) {
      if (stopped) return;
      for (const draft of drafts) receive(draft);
      const known = new Set(drafts.map(draftId));
      for (const local of device.list()) if (local.text && !known.has(draftId(local))) markDirty(entry(local), true);
      for (const item of entries.values()) if (item.dirty) void save(item);
    },
    stop() {
      stopped = true;
      for (const item of entries.values()) clearTimeout(item.timer);
    },
  };
}

export type DraftSync = ReturnType<typeof createDraftSync>;
