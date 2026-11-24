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
