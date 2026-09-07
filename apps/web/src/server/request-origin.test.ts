import assert from "node:assert/strict";
import test from "node:test";
import { sameOrigin } from "./request-origin.ts";

test("cookie mutations require configured exact origin, never cross-site", () => {
  const request = (origin?: string, site = "same-origin") => new Request("http://internal:3000/api/account/profile", {
    method: "POST", headers: { ...(origin ? { origin } : {}), "sec-fetch-site": site },
  });
  assert.equal(sameOrigin(request("http://internal:3000")), true);
  assert.equal(sameOrigin(request("https://evil.example")), false);
  assert.equal(sameOrigin(request()), false);
  assert.equal(sameOrigin(request("http://internal:3000", "cross-site")), false);
});
