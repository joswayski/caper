import assert from "node:assert/strict";
import { test } from "node:test";
import { routeOutput } from "../audio/output.ts";
import { rosterChanges } from "../audio/roster.ts";

test("roster changes report other people joining and leaving, never self", () => {
  const ids = (...values: string[]) => new Set(values);
  assert.deepEqual(rosterChanges(undefined, ids("me", "a"), "me"), { joined: false, left: false });
  assert.deepEqual(rosterChanges(ids("me"), ids("me", "a"), "me"), { joined: true, left: false });
  assert.deepEqual(rosterChanges(ids("me", "a"), ids("me"), "me"), { joined: false, left: true });
  assert.deepEqual(rosterChanges(ids("a"), ids("me", "a"), "me"), { joined: false, left: false });
  assert.deepEqual(rosterChanges(ids("me", "a"), ids("me", "b"), "me"), { joined: true, left: true });
});

test("output routing falls back to the system default before reporting failure", async () => {
  const calls: string[] = [];
  const element = (failing: Set<string>) => ({
    setSinkId: async (id: string) => { calls.push(id); if (failing.has(id)) throw new DOMException("no", "NotAllowedError"); },
  });
  assert.equal(await routeOutput({}, "speaker"), true);
  assert.equal(await routeOutput(element(new Set()), "speaker"), true);
  assert.deepEqual(calls.splice(0), ["speaker"]);
  assert.equal(await routeOutput(element(new Set(["speaker"])), "speaker"), true);
  assert.deepEqual(calls.splice(0), ["speaker", ""]);
  assert.equal(await routeOutput(element(new Set(["speaker", ""])), "speaker"), false);
  assert.deepEqual(calls.splice(0), ["speaker", ""]);
  assert.equal(await routeOutput(element(new Set([""])), ""), false);
  assert.deepEqual(calls.splice(0), [""]);
});
