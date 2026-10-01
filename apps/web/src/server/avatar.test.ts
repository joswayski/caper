import assert from "node:assert/strict";
import { test } from "node:test";
import { avatarUrl } from "../account/avatar.ts";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { callSnapshot } from "../media/events.ts";

test("all client vector resources match the canonical masters and persisted ID mapping", () => {
  execFileSync(process.execPath, [new URL("../../../../scripts/generate-avatar-vectors.mjs", import.meta.url).pathname, "--check"]);
});

test("all saved IDs have distinct path-only SVG artwork, not embedded rasters", () => {
  for (let id = 0; id < 800; id++) {
    assert.equal(avatarUrl(id), `/images/avatars/v2/${id}.svg`);
    const svg = readFileSync(new URL(`../../public/images/avatars/v2/${id}.svg`, import.meta.url), "utf8");
    assert.match(svg, /viewBox="0 0 256 256"/);
    assert.match(svg, /<path d=/);
    assert.doesNotMatch(svg, /<image|data:|href=/i);
  }
});

test("old API data and invalid saved IDs use initials, never an arbitrary picture", () => {
  for (const id of [undefined, null, -1, 800, 1.5, NaN, Infinity]) assert.equal(avatarUrl(id), undefined);
});

test("active and spectator voice retain saved avatar even when call identity changes", () => {
  const person = { id: "first-call", name: "Same name", avatarId: 255, muted: false, deafened: false };
  const active = callSnapshot({ type: "snapshot", participants: [{ ...person, tracks: [] }], revision: 1 }, false);
  const spectator = callSnapshot({ type: "snapshot", participants: [{ ...person, id: "new-call" }], revision: 2 }, true);
  assert.equal(active.participants[0].avatarId, 255);
  assert.equal(spectator.participants[0].avatarId, 255);
  assert.deepEqual(spectator.participants[0].tracks, []);
});
