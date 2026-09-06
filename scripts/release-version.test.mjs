import assert from "node:assert/strict";
import test from "node:test";

import {
  configuredReleaseDate,
  nextReleaseVersion,
  releaseDate,
} from "./release-version.mjs";

test("uses the New York calendar date across UTC midnight", () => {
  assert.equal(releaseDate(new Date("2026-07-20T01:30:00Z")), "2026-07-19");
  assert.equal(releaseDate(new Date("2026-07-20T04:30:00Z")), "2026-07-20");
});

test("creates updater-safe versions and increments same-day releases", () => {
  assert.deepEqual(nextReleaseVersion("2026-07-19", []), {
    displayVersion: "2026.07.19.1",
    tag: "v2026.07.19.1",
    appVersion: "2026.7.1901",
  });
  assert.equal(
    nextReleaseVersion("2026-07-19", ["v2026.07.19.1", "v2026.07.19.3"]).appVersion,
    "2026.7.1904",
  );
});

test("uses the batched main commit time for delayed releases", () => {
  assert.equal(configuredReleaseDate("2026-07-20T01:30:00Z"), "2026-07-19");
  assert.throws(() => configuredReleaseDate("not-a-date"), /ISO-8601 timestamp/u);
});

test("rejects invalid dates and malformed matching tags", () => {
  assert.throws(() => nextReleaseVersion("2026-02-30", []), /real calendar date/u);
  assert.throws(
    () => nextReleaseVersion("2026-07-19", ["v2026.07.19.beta"]),
    /malformed Caper release tag/u,
  );
});
