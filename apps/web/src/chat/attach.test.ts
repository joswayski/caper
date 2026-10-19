import { describe, expect, it } from "vitest";
import { attachCapacity } from "./attach.ts";

describe("attachCapacity", () => {
  it("describes an empty, partly used and full draft", () => {
    expect(attachCapacity(0)).toEqual({ count: 0, full: false, label: "Up to 10 files", hint: undefined });
    expect(attachCapacity(3)).toEqual({ count: 3, full: false, label: "3 of 10 attached", hint: undefined });
    expect(attachCapacity(10)).toEqual({
      count: 10,
      full: true,
      label: "10 of 10 attached",
      hint: "Remove a file to add another.",
    });
  });

  it("clamps counts outside the limit", () => {
    expect(attachCapacity(12).count).toBe(10);
    expect(attachCapacity(-1).count).toBe(0);
  });
});
