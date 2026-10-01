import assert from "node:assert/strict";
import { test } from "node:test";
import { avatarPosition } from "../account/avatar.ts";
import { callSnapshot } from "../media/events.ts";

test("saved avatar IDs select row-major tiles, including zero and row boundaries", () => {
  assert.equal(avatarPosition(0), "0% 0%");
  assert.equal(avatarPosition(31), "100% 0%");
  assert.equal(avatarPosition(32), "0% 4.166666666666667%");
  assert.equal(avatarPosition(798), "96.7741935483871% 100%");
  assert.equal(avatarPosition(799), "100% 100%");
  assert.equal(new Set(Array.from({ length: 800 }, (_, id) => avatarPosition(id))).size, 800);
});

test("old API data and invalid saved IDs use initials, never an arbitrary tile", () => {
  for (const id of [undefined, null, -1, 800, 1.5, NaN, Infinity]) assert.equal(avatarPosition(id), undefined);
});

test("active and spectator voice retain saved avatar even when call identity changes", () => {
  const person = { id: "first-call", name: "Same name", avatarId: 255, muted: false, deafened: false };
  const active = callSnapshot({ type: "snapshot", participants: [{ ...person, tracks: [] }], revision: 1 }, false);
  const spectator = callSnapshot({ type: "snapshot", participants: [{ ...person, id: "new-call" }], revision: 2 }, true);
  assert.equal(active.participants[0].avatarId, 255);
  assert.equal(spectator.participants[0].avatarId, 255);
  assert.deepEqual(spectator.participants[0].tracks, []);
});
