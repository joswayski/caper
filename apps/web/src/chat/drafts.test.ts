import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createDraftSync,
  draftId,
  filesOf,
  type DeviceDrafts,
  type DraftFiles,
  type DraftRef,
  type DraftSave,
} from "./draft-sync.ts";
import { showStagedFiles, stagedFileChanged, stagedUrls, stashStagedFiles, takeStagedFiles } from "./drafts.ts";
import type { AttachmentUpload } from "./uploads.ts";

const chip = (key: string) => ({ key, name: `${key}.png`, sourceSize: 4, progress: 0, localUrl: `blob:${key}` });

describe("staged files per conversation", () => {
  afterEach(() => {
    showStagedFiles(undefined);
    takeStagedFiles("a:one");
    takeStagedFiles("a:two");
  });

  it("keeps each conversation's files until it is reopened", () => {
    const uploads = new Map([["x", {} as AttachmentUpload]]);
    stashStagedFiles("a:one", { drafts: [chip("x")], uploads });
    stashStagedFiles("a:two", { drafts: [chip("y")], uploads: new Map() });
    expect(stagedUrls()).toEqual(new Set(["blob:x", "blob:y"]));
    const one = takeStagedFiles("a:one");
    expect(one?.drafts.map((draft) => draft.key)).toEqual(["x"]);
    expect(one?.uploads).toBe(uploads);
    expect(takeStagedFiles("a:one")).toBeUndefined();
    expect(takeStagedFiles("a:two")?.drafts.map((draft) => draft.key)).toEqual(["y"]);
  });

  it("does not keep an empty conversation", () => {
    stashStagedFiles("a:one", { drafts: [chip("x")], uploads: new Map() });
    stashStagedFiles("a:one", { drafts: [], uploads: new Map() });
    expect(takeStagedFiles("a:one")).toBeUndefined();
  });

  it("records progress for stashed files and tells the open conversation", () => {
    const visible = vi.fn();
    showStagedFiles(visible);
    stashStagedFiles("a:one", { drafts: [chip("x")], uploads: new Map() });
    stagedFileChanged("x", { progress: 0.5 });
    expect(visible).toHaveBeenCalledWith("x", { progress: 0.5 });
    expect(takeStagedFiles("a:one")?.drafts[0].progress).toBe(0.5);
  });
});

// The rules in docs/media.md ("Draft sync"), against a device copy and a server stub.
describe("draft sync", () => {
  const channel = { channelId: "chan00000001", threadRootId: null };
  const stamp = (n: number) => `2026-11-27T18:04:05.${String(n).padStart(6, "0")}Z`;

  const file = (id: string, spoiler = false) => ({
    id,
    kind: "image" as const,
    contentType: "image/png",
    name: `${id}.png`,
    size: 4,
    url: `https://cdn.example/${id}`,
    ...(spoiler ? { spoiler } : {}),
  });

  function setup(initial: Record<string, string> = {}) {
    const copy = new Map(Object.entries(initial));
    /** Uploaded files per draft; a missing entry is "unknown here". */
    const files = new Map<string, DraftFiles>();
    const dirty = new Set<string>();
    const device: DeviceDrafts = {
      read: (ref) => ({ text: copy.get(draftId(ref)) ?? "", files: files.get(draftId(ref)) }),
      write: (draft) => {
        copy.set(draftId(draft), draft.text);
        files.set(draftId(draft), filesOf(draft.attachments));
      },
      list: () =>
        [...new Set([...copy.keys(), ...files.keys()])].flatMap((id) => {
          const [channelId, threadRootId] = id.split(" ");
          const local = { text: copy.get(id) ?? "", files: files.get(id) };
          return local.text || local.files?.ids.length
            ? [{ channelId, threadRootId: threadRootId ?? null, ...local }]
            : [];
        }),
      dirtyIds: () => [...dirty],
      setDirty: (id, value) => void (value ? dirty.add(id) : dirty.delete(id)),
    };
    let clock = 10;
    // Like the API: the saved draft lists the files it was given.
    const save = vi.fn(async ({ attachmentIds, spoilerAttachmentIds, ...draft }: DraftSave) => ({
      ...draft,
      attachments: (attachmentIds ?? []).map((id) => file(id, !!spoilerAttachmentIds?.includes(id))),
      updatedAt: stamp(++clock),
    }));
    const onApplied = vi.fn();
    const sync = createDraftSync({ api: { save }, device, onApplied });
    /** A local edit, as a composer makes it: device copy first, then the sync. */
    const type = (text: string, ref: DraftRef = channel) => {
      copy.set(draftId(ref), text);
      sync.edited(ref);
    };
    /** A staged file finished uploading, was removed or re-marked here. */
    const stage = (next: DraftFiles, ref: DraftRef = channel) => {
      files.set(draftId(ref), next);
      sync.edited(ref);
    };
    return { copy, files, dirty, save, onApplied, sync, type, stage };
  }

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("saves about a second after typing stops, and at once on flush or clear", async () => {
    const { save, sync, type } = setup();
    type("h");
    await vi.advanceTimersByTimeAsync(600);
    type("hi");
    await vi.advanceTimersByTimeAsync(600);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(save).toHaveBeenCalledExactlyOnceWith({ ...channel, text: "hi" });
    type("hi there");
    sync.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith({ ...channel, text: "hi there" });
    // Sending clears the draft, which saves empty text without waiting.
    type("");
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith({ ...channel, text: "" });
    expect(save).toHaveBeenCalledTimes(3);
  });

  it("saves one draft at a time, in order", async () => {
    const { save, sync, type } = setup();
    let finish!: () => void;
    save.mockImplementationOnce(
      (draft) => new Promise((resolve) => (finish = () => resolve({ ...draft, attachments: [], updatedAt: stamp(1) }))),
    );
    type("a");
    sync.flush();
    type("ab");
    sync.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenCalledTimes(1);
    finish();
    await vi.advanceTimersByTimeAsync(0);
    expect(save.mock.calls.map(([draft]) => draft.text)).toEqual(["a", "ab"]);
  });

  it("does not overwrite a dirty draft, and a failed save stays dirty until it retries", async () => {
    const { copy, dirty, save, onApplied, sync, type } = setup();
    save.mockRejectedValueOnce(new Error("offline"));
    type("mine");
    sync.received({ ...channel, text: "theirs", attachments: [], updatedAt: stamp(50) });
    expect(copy.get(channel.channelId)).toBe("mine");
    sync.flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(dirty.has(channel.channelId)).toBe(true);
    sync.received({ ...channel, text: "theirs", attachments: [], updatedAt: stamp(51) });
    expect(copy.get(channel.channelId)).toBe("mine");
    expect(onApplied).not.toHaveBeenCalled();
    // A reconnect's GET retries it.
    sync.loaded([{ ...channel, text: "theirs", attachments: [], updatedAt: stamp(51) }]);
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith({ ...channel, text: "mine" });
    expect(dirty.size).toBe(0);
  });

  it("applies newer server drafts and ignores ones not newer than the last applied", () => {
    const { copy, onApplied, sync } = setup({ [channel.channelId]: "old" });
    sync.received({ ...channel, text: "second", attachments: [], updatedAt: stamp(2) });
    sync.received({ ...channel, text: "first", attachments: [], updatedAt: stamp(1) });
    sync.received({ ...channel, text: "again", attachments: [], updatedAt: stamp(2) });
    expect(copy.get(channel.channelId)).toBe("second");
    expect(onApplied).toHaveBeenCalledExactlyOnceWith(channel, "second");
    // Cleared on another device clears it here.
    sync.received({ ...channel, text: "", attachments: [], updatedAt: stamp(3) });
    expect(copy.get(channel.channelId)).toBe("");
  });

  it("treats a save's response as applied, so its own echo changes nothing", async () => {
    const { copy, onApplied, save, sync, type } = setup();
    type("hello");
    await vi.advanceTimersByTimeAsync(1000);
    const saved = await save.mock.results[0].value;
    sync.received(saved);
    // An echo of an earlier save, delivered late, is older still.
    sync.received({ ...channel, text: "hel", attachments: [], updatedAt: stamp(5) });
    expect(copy.get(channel.channelId)).toBe("hello");
    expect(onApplied).not.toHaveBeenCalled();
  });

  it("uploads device-only drafts after GET, and only those", async () => {
    const thread = { channelId: "chan00000002", threadRootId: "root00000000001" };
    const { save, sync } = setup({
      [channel.channelId]: "typed offline",
      [draftId(thread)]: "a reply",
      chan00000003: "already synced",
    });
    sync.loaded([
      { channelId: "chan00000003", threadRootId: null, text: "already synced", attachments: [], updatedAt: stamp(1) },
    ]);
    await vi.advanceTimersByTimeAsync(0);
    expect(save.mock.calls.map(([draft]) => draft)).toEqual([
      { ...channel, text: "typed offline" },
      { ...thread, text: "a reply" },
    ]);
  });

  it("saves uploaded files by ID in tile order with their spoiler marks, as an edit like typing", async () => {
    const { save, stage, type } = setup();
    stage({ ids: ["file0000001", "file0000002"], spoilers: ["file0000002"] });
    await vi.advanceTimersByTimeAsync(999);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(save).toHaveBeenCalledExactlyOnceWith({
      ...channel,
      text: "",
      attachmentIds: ["file0000001", "file0000002"],
      spoilerAttachmentIds: ["file0000002"],
    });
    // Removing the last file (and no text) clears the draft at once.
    stage({ ids: [], spoilers: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith({ ...channel, text: "", attachmentIds: [], spoilerAttachmentIds: [] });
    // Text typed where this device knows nothing about files leaves the server's alone.
    const other = { channelId: "chan00000009", threadRootId: null };
    type("just text", other);
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenLastCalledWith({ ...other, text: "just text" });
  });

  it("applies a server draft's files to the device copy, and its own echo with files changes nothing", async () => {
    const { files, onApplied, save, sync, stage } = setup();
    sync.received({ ...channel, text: "look", attachments: [file("file0000001", true)], updatedAt: stamp(1) });
    expect(files.get(channel.channelId)).toEqual({ ids: ["file0000001"], spoilers: ["file0000001"] });
    expect(onApplied).toHaveBeenCalledTimes(1);
    stage({ ids: ["file0000001", "file0000002"], spoilers: [] });
    await vi.advanceTimersByTimeAsync(1000);
    sync.received(await save.mock.results[0].value);
    expect(files.get(channel.channelId)).toEqual({ ids: ["file0000001", "file0000002"], spoilers: [] });
    expect(onApplied).toHaveBeenCalledTimes(1);
  });

  it("uploads a device-only draft that has files but no text after GET", async () => {
    const { files, save, sync } = setup();
    files.set(channel.channelId, { ids: ["file0000001"], spoilers: [] });
    sync.loaded([]);
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenCalledExactlyOnceWith({
      ...channel,
      text: "",
      attachmentIds: ["file0000001"],
      spoilerAttachmentIds: [],
    });
  });
});
