import { describe, expect, it } from "vitest";
import { attachCapacity } from "./attach.ts";

describe("attachCapacity", () => {
  it("says nothing until the draft is full", () => {
    expect(attachCapacity(0)).toEqual({ full: false, hint: undefined });
    expect(attachCapacity(9)).toEqual({ full: false, hint: undefined });
  });

  it("explains how to make room without a redundant count", () => {
    expect(attachCapacity(10)).toEqual({
      full: true,
      hint: "Remove a file to add another.",
    });
    expect(attachCapacity(12).hint).toBe("Remove a file to add another.");
  });
});
