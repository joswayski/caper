import assert from "node:assert/strict";
import { test } from "vitest";
import { routeOutput } from "../audio/output.ts";

test("output routing falls back to the system default before reporting failure", async () => {
  const calls: string[] = [];
  const element = (failing: Set<string>) => ({
    setSinkId: async (id: string) => {
      calls.push(id);
      if (failing.has(id)) throw new DOMException("no", "NotAllowedError");
    },
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
