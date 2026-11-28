import { expect, test, vi } from "vitest";
import type { DraftAttachment } from "../chat/Attachments.tsx";
import type { DraftSave } from "../chat/draft-sync.ts";
import type { AttachmentUpload } from "../chat/uploads.ts";
import { readDraft, saveDraft } from "../chat/drafts.ts";

test("each conversation keeps its own unsent draft until it is cleared", () => {
  saveDraft("account:general", "first draft");
  expect(readDraft("account:design")).toBe("");
  saveDraft("account:design", "design draft");
  expect(readDraft("account:general")).toBe("first draft");
  saveDraft("account:general", "");
  expect(readDraft("account:general")).toBe("");
  expect(readDraft("account:design")).toBe("design draft");
  expect(readDraft("other:design")).toBe("");
});

function stubStorage() {
  const items = new Map<string, string>();
  const storage = {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
  // Object.keys(localStorage) lists the stored keys, as in browsers.
  vi.stubGlobal(
    "localStorage",
    new Proxy(storage, {
      ownKeys: () => [...items.keys()],
      getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
    }),
  );
  return items;
}

/** A reload: the module's in-memory copies are gone, storage stays. */
async function reload() {
  vi.resetModules();
  return import("../chat/drafts.ts");
}

test("drafts and thread replies survive a reload per account, and logout erases only that account's", async () => {
  const items = stubStorage();
  const before = await reload();
  before.saveDraft("alice:general", "half a thought");
  before.saveDraft("alice:design", "sent");
  before.saveDraft("alice:design", "");
  before.saveDraft("bob:general", "bob's words");
  before.saveThreadDrafts("alice", {
    root1: { text: "a reply", broadcast: true },
    root2: { text: "", broadcast: false },
  });
  before.saveThreadDrafts("bob", { root1: { text: "bob's reply", broadcast: false } });

  const after = await reload();
  expect(after.readDraft("alice:general")).toBe("half a thought");
  expect(after.readDraft("alice:design")).toBe("");
  expect(after.readThreadDrafts("alice")).toEqual({ root1: { text: "a reply", broadcast: true } });
  // The signed-out simulation neither reads nor keeps saved replies.
  expect(after.readThreadDrafts(undefined)).toEqual({});

  after.forgetDrafts("alice");
  const next = await reload();
  expect(next.readDraft("alice:general")).toBe("");
  expect(next.readThreadDrafts("alice")).toEqual({});
  expect(next.readDraft("bob:general")).toBe("bob's words");
  expect(next.readThreadDrafts("bob")).toEqual({ root1: { text: "bob's reply", broadcast: false } });
  expect([...items.keys()].some((key) => key.includes("alice"))).toBe(false);
});

test("server drafts reach the device copy and open composers; thread replies keep their broadcast choice", async () => {
  stubStorage();
  const drafts = await reload();
  drafts.saveDraft("alice:chan00000001", "local");
  drafts.saveThreadDrafts("alice", {
    root00000000001: { text: "my reply", broadcast: true, channelId: "chan00000001" },
  });
  const save = vi.fn(async (draft: { channelId: string; threadRootId: string | null; text: string }) => ({
    ...draft,
    attachments: [],
    updatedAt: "2026-11-27T18:04:05.000001Z",
  }));
  const applied = vi.fn();
  const stop = drafts.watchServerDrafts(applied);
  const { disconnect } = drafts.connectDraftSync("alice", { save });
  drafts.receiveServerDraft({
    channelId: "chan00000001",
    threadRootId: "root00000000001",
    text: "from the phone",
    updatedAt: "2026-11-27T18:04:06.000000Z",
  });
  drafts.receiveServerDraft({ channelId: 7, text: "malformed" });
  expect(drafts.readThreadDrafts("alice").root00000000001).toEqual({
    text: "from the phone",
    broadcast: true,
    channelId: "chan00000001",
  });
  expect(drafts.readDraft("alice:chan00000001")).toBe("local");
  expect(applied).toHaveBeenCalledWith({
    accountId: "alice",
    channelId: "chan00000001",
    threadRootId: "root00000000001",
    text: "from the phone",
  });
  stop();
  disconnect();
  expect(save).not.toHaveBeenCalled();
});

test("the signed-out simulation neither keeps nor syncs a thread reply", async () => {
  const items = stubStorage();
  const drafts = await reload();
  const save = vi.fn();
  const { disconnect } = drafts.connectDraftSync("alice", { save });
  drafts.editThreadDraft(undefined, "chan00000001", "root00000000001", { text: "hi", broadcast: false });
  drafts.flushDraftSync();
  expect(save).not.toHaveBeenCalled();
  expect(items.size).toBe(0);
  disconnect();
});

// Rule 4 of docs/media.md ("Draft sync"): staged files sync once uploaded.
const file = (id: string, spoiler = false) => ({
  id,
  kind: "image" as const,
  contentType: "image/png",
  name: `${id}.png`,
  size: 4,
  url: `https://cdn.example/${id}`,
  previewUrl: `https://cdn.example/${id}/preview`,
  ...(spoiler ? { spoiler } : {}),
});
/** Like the API: the saved draft lists the files it was given. */
const echoingSave = () => {
  let clock = 0;
  return vi.fn(async ({ attachmentIds, spoilerAttachmentIds, ...draft }: DraftSave) => ({
    ...draft,
    attachments: (attachmentIds ?? []).map((id) => file(id, !!spoilerAttachmentIds?.includes(id))),
    updatedAt: `2026-11-27T18:04:05.${String(++clock).padStart(6, "0")}Z`,
  }));
};
const tile = (key: string, change: Partial<DraftAttachment> = {}): DraftAttachment => ({
  key,
  name: `${key}.png`,
  sourceSize: 4,
  progress: 0.5,
  ...change,
});
/** An upload this device started: still going, or finished as `attachment`. */
const ownUpload = (attachment?: ReturnType<typeof file>) =>
  ({
    state: { name: "x", sourceSize: 4, progress: attachment ? 1 : 0.5, attachment },
    release: vi.fn(),
  }) as unknown as AttachmentUpload & { release: ReturnType<typeof vi.fn> };

test("a finished upload saves the draft with its ID; a file still uploading is not sent", async () => {
  vi.useFakeTimers();
  try {
    stubStorage();
    const drafts = await reload();
    const save = echoingSave();
    const { disconnect } = drafts.connectDraftSync("alice", { save });
    const uploading = tile("a");
    drafts.reportStagedFiles("alice:chan00000001", [uploading]);
    await vi.advanceTimersByTimeAsync(2000);
    expect(save).not.toHaveBeenCalled();
    const finished = tile("b", { progress: 1, attachment: file("file0000001"), spoiler: true });
    drafts.reportStagedFiles("alice:chan00000001", [uploading, finished]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledExactlyOnceWith({
      channelId: "chan00000001",
      threadRootId: null,
      text: "",
      attachmentIds: ["file0000001"],
      spoilerAttachmentIds: ["file0000001"],
    });
    // Its own echo leaves the open conversation's tiles alone.
    const applied = vi.fn();
    const hide = drafts.showDraftFiles("alice:chan00000001", applied);
    drafts.receiveServerDraft(await save.mock.results[0].value);
    expect(applied).not.toHaveBeenCalled();
    // Removing it is an edit too; with no text that clears the draft at once.
    drafts.reportStagedFiles("alice:chan00000001", [uploading]);
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith({
      channelId: "chan00000001",
      threadRootId: null,
      text: "",
      attachmentIds: [],
      spoilerAttachmentIds: [],
    });
    hide();
    disconnect();
  } finally {
    vi.useRealTimers();
  }
});

test("a server draft's files become ready tiles; files still uploading stay and unlisted ones go", async () => {
  stubStorage();
  const drafts = await reload();
  const fetch = vi.fn(async () => new Response(null, { status: 204 }));
  vi.stubGlobal("fetch", fetch);
  const { disconnect } = drafts.connectDraftSync("alice", { save: echoingSave() });
  const uploading = ownUpload();
  const removedElsewhere = ownUpload(file("file0000009"));
  const thread = tile("t", { threadRootId: "root00000000001", progress: 1, attachment: file("file0000008") });
  drafts.stashStagedFiles("alice:chan00000001", {
    drafts: [tile("a"), tile("b", { progress: 1, attachment: file("file0000009") }), thread],
    uploads: new Map([
      ["a", uploading],
      ["b", removedElsewhere],
      ["t", ownUpload(file("file0000008"))],
    ]),
  });
  drafts.receiveServerDraft({
    channelId: "chan00000001",
    threadRootId: null,
    text: "look at these",
    attachments: [file("file0000001", true)],
    updatedAt: "2026-11-27T18:04:06.000000Z",
  });
  expect(drafts.readDraft("alice:chan00000001")).toBe("look at these");
  const staged = drafts.takeStagedFiles("alice:chan00000001")!;
  const channelTiles = staged.drafts.filter((draft) => !draft.threadRootId);
  expect(channelTiles.map((draft) => draft.attachment?.id ?? draft.key)).toEqual(["file0000001", "a"]);
  const synced = channelTiles[0];
  expect(synced).toMatchObject({ progress: 1, spoiler: true, attachment: file("file0000001", true) });
  expect(synced.localUrl).toBeUndefined();
  // The thread's files are another draft's.
  expect(staged.drafts).toContainEqual(thread);
  // The file another device removed (or sent) is released here; the uploading one is kept.
  expect(removedElsewhere.release).toHaveBeenCalledOnce();
  expect(removedElsewhere.release).not.toHaveBeenCalledWith(true);
  expect(staged.uploads.has("b")).toBe(false);
  expect(staged.uploads.get("a")).toBe(uploading);
  // A synced tile sends by its ID, and this device never cancels it.
  const upload = staged.uploads.get(synced.key)!;
  expect(upload.synced).toBe(true);
  expect(upload.stale).toBe(false);
  await expect(upload.ensure()).resolves.toMatchObject({ id: "file0000001" });
  upload.release(true);
  expect(fetch).not.toHaveBeenCalled();
  disconnect();
});
