import { describe, expect, it } from "vitest";
import { attachCapacity } from "./attach.ts";

describe("attachCapacity", () => {
  it("says nothing until the draft is full", () => {
    expect(attachCapacity(0)).toEqual({ full: false, label: undefined, hint: undefined });
    expect(attachCapacity(9)).toEqual({ full: false, label: undefined, hint: undefined });
  });

  it("names the limit and how to make room once full", () => {
    expect(attachCapacity(10)).toEqual({
      full: true,
      label: "10 of 10 attached",
      hint: "Remove a file to add another.",
    });
    expect(attachCapacity(12).label).toBe("10 of 10 attached");
  });
});
