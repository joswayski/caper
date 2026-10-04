import assert from "node:assert/strict";
import { test } from "node:test";
import { sessionDuration } from "../media/session-duration.ts";

test("voice duration uses the shared start, clamps clock skew and keeps hours", () => {
  const start = 1_734_567_890_123;
  for (const [elapsed, expected] of [
    [-1_001, "00:00"], [999, "00:00"], [1_000, "00:01"],
    [59_999, "00:59"], [60_000, "01:00"], [1_701_000, "28:21"],
    [3_599_999, "59:59"], [3_600_000, "1:00:00"], [7_384_000, "2:03:04"],
    [360_001_000, "100:00:01"],
  ] as const) {
    assert.equal(sessionDuration(start, start + elapsed), expected);
  }
});
