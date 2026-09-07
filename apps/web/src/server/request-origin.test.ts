import assert from "node:assert/strict";
import test from "node:test";
import { sameOrigin } from "./request-origin.ts";

test("cookie mutations require configured exact origin, never cross-site", (t) => {
  const previous = process.env.WORKOS_REDIRECT_URI;
  t.after(() => { if (previous) process.env.WORKOS_REDIRECT_URI = previous; else delete process.env.WORKOS_REDIRECT_URI; });
  process.env.WORKOS_REDIRECT_URI = "https://caper.chat/api/auth/callback";
  const request = (origin?: string, site = "same-origin") => new Request("http://internal:3000/api/account/profile", {
    method: "POST", headers: { ...(origin ? { origin } : {}), "sec-fetch-site": site },
  });
  assert.equal(sameOrigin(request("https://caper.chat")), true);
  assert.equal(sameOrigin(request("https://evil.example")), false);
  assert.equal(sameOrigin(request()), false);
  assert.equal(sameOrigin(request("https://caper.chat", "cross-site")), false);
  delete process.env.WORKOS_REDIRECT_URI;
  assert.equal(sameOrigin(request("https://caper.chat")), false);
});
