import { expect, test } from "vitest";
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
