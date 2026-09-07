import assert from "node:assert/strict";
import test from "node:test";
import { proxyAccount } from "./account-proxy.ts";

test("browser profile proxy bounds input and sends only verified Bearer identity to Rust", async (t) => {
  const previous = process.env.MEDIA_API_URL;
  process.env.MEDIA_API_URL = "http://api:3001";
  t.after(() => { if (previous) process.env.MEDIA_API_URL = previous; else delete process.env.MEDIA_API_URL; });
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "http://api:3001/api/account/profile");
    assert.equal(new Headers(init.headers).get("authorization"), "Bearer verified-jwt");
    assert.equal(new Headers(init.headers).get("cookie"), null);
    assert.equal(new Headers(init.headers).get("x-caper-account-token"), null);
    return Response.json({ username: "caper_test" });
  });
  const request = (body = "{}") => new Request("https://caper.chat/api/account/profile", { method: "POST", headers: {
    "content-type": "application/json", authorization: "Bearer attacker", cookie: "wos-session=fixture",
    "x-caper-account-token": "attacker",
  }, body });
  assert.equal((await proxyAccount(request(), "verified-jwt")).status, 200);
  assert.equal((await proxyAccount(request(), "")).status, 401);
  assert.equal((await proxyAccount(request("x".repeat(4097)), "verified-jwt")).status, 413);
  assert.equal((await proxyAccount(new Request("https://caper.chat/api/account/profile"), "verified-jwt")).status, 405);
});
