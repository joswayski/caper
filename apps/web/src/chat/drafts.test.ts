import { afterEach, describe, expect, it, vi } from "vitest";
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
