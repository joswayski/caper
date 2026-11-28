import { expect, test, vi } from "vitest";
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
