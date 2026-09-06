import assert from "node:assert/strict";
import test from "node:test";
import { getHealth } from "./api.ts";

test("health endpoint reports readiness without caching", async () => {
  const response = getHealth(new Request("http://localhost/api/health"));

  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), { status: "ok" });
});
