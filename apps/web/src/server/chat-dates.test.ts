import assert from "node:assert/strict";
import { test } from "vitest";
import { dateDivider } from "../chat/dates.ts";

test("date dividers follow local midnight, not UTC midnight or elapsed hours", () => {
  const previousTimezone = process.env.TZ;
  process.env.TZ = "America/New_York";
  try {
    assert.ok(dateDivider("2026-09-29T03:59:00Z")?.includes("28"));
    assert.equal(dateDivider("2026-09-29T03:59:00Z", "2026-09-28T23:00:00Z"), undefined);
    assert.ok(dateDivider("2026-09-29T04:00:00Z", "2026-09-29T03:59:00Z")?.includes("29"));
    assert.equal(dateDivider("2026-11-01T06:30:00Z", "2026-11-01T05:30:00Z"), undefined);
    assert.ok(dateDivider("2027-01-01T05:00:00Z", "2027-01-01T04:59:00Z")?.includes("2027"));
    // Prepending an earlier same-day message moves the divider to that message.
    assert.ok(dateDivider("2026-09-29T10:00:00Z"));
    assert.equal(dateDivider("2026-09-29T12:00:00Z", "2026-09-29T10:00:00Z"), undefined);
    assert.equal(dateDivider("invalid"), undefined);
  } finally {
    if (previousTimezone === undefined) delete process.env.TZ;
    else process.env.TZ = previousTimezone;
  }
});
