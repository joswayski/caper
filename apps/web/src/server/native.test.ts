import assert from "node:assert/strict";
import test from "node:test";
import { proxyNative } from "./native.ts";

test("native gateway accepts only explicit account tokens, not cookies", async () => {
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/account/me", { headers: { cookie: "wos-session=fixture" } }))).status, 401);
});

test("native gateway bounds paths, methods and payloads and never forwards cookies", async (t) => {
  const previous = process.env.MEDIA_API_URL;
  process.env.MEDIA_API_URL = "http://media:3001";
  t.after(() => { if (previous) process.env.MEDIA_API_URL = previous; else delete process.env.MEDIA_API_URL; });
  const headers = { "x-caper-account-token": "jwt-fixture", cookie: "wos-session=unrelated", "content-type": "application/json" };
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "http://media:3001/api/account/me");
    assert.equal(new Headers(init.headers).get("x-caper-account-token"), "jwt-fixture");
    assert.equal(new Headers(init.headers).get("cookie"), null);
    return Response.json({ id: "public-id", username: null, displayName: null });
  });
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/account/me", { headers }))).status, 200);
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/account/me", { method: "POST", headers, body: "{}" }))).status, 405);
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/account/profile", { method: "POST", headers, body: "x".repeat(4097) }))).status, 413);
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/media/secrets", { headers }))).status, 404);
  assert.equal((await proxyNative(new Request("https://caper.chat/api/native/https://evil.example", { headers }))).status, 404);
});
