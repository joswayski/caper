import assert from "node:assert/strict";
import test from "node:test";
import { getWebHealth } from "./health.ts";

test("health endpoint reports readiness without caching", async () => {
  const response = getWebHealth(new Request("http://localhost/health"));

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), { status: "ok" });
});
