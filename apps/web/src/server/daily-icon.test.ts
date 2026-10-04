import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dailyIcon } from "../components/daily-icon.ts";

test("wordmark exports preserve the original lettering and omit the static dot", () => {
  execFileSync(process.execPath, [new URL("../../../../scripts/generate-wordmark.mjs", import.meta.url).pathname, "--check"]);
  const original = readFileSync(new URL("../../public/caper-wordmark.svg", import.meta.url), "utf8");
  const lettering = readFileSync(new URL("../../public/caper-wordmark-letters.svg", import.meta.url), "utf8");
  assert.equal((lettering.match(/<path\b/g) ?? []).length, 1);
  assert.equal(lettering.match(/<path[^>]+>/)?.[0], original.match(/<path[^>]+>/)?.[0]);
  assert.match(lettering, /viewBox="20 17 1042 276"/);
});

test("daily icon remains stable until the UTC boundary", () => {
  const saved = { day: 0, index: 143 };
  assert.deepEqual(dailyIcon(saved, 86_399_999, () => { throw Error("must not draw"); }), saved);
  assert.deepEqual(dailyIcon(saved, 86_400_000, () => 0), { day: 1, index: 0 });
});

test("daily icon selects all 800 avatars and excludes the previous one without losing the last index", () => {
  assert.deepEqual(new Set(Array.from({ length: 800 }, (_, i) => dailyIcon(null, 0, () => (i + 0.5) / 800).index)), new Set(Array.from({ length: 800 }, (_, i) => i)));
  for (const previous of [0, 143, 798, 799]) {
    const selected = new Set(Array.from({ length: 799 }, (_, i) => dailyIcon({ day: 0, index: previous }, 86_400_000, () => (i + 0.5) / 799).index));
    assert.equal(selected.size, 799);
    assert.equal(selected.has(previous), false);
    assert.equal(selected.has(799), previous !== 799);
  }
});

test("invalid saved IDs do not pin an invalid icon on the current day", () => {
  for (const index of [-1, 800, 1.5, NaN]) assert.deepEqual(dailyIcon({ day: 0, index }, 0, () => 0), { day: 0, index: 0 });
});
