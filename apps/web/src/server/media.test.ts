import assert from "node:assert/strict";
import test from "node:test";
import { proxyMedia } from "./media.ts";

test("adapter is disabled without configuration and restricts operations/methods", async (t) => {
  const old = process.env.MEDIA_API_URL;
  delete process.env.MEDIA_API_URL;
  t.after(() => { if (old) process.env.MEDIA_API_URL = old; else delete process.env.MEDIA_API_URL; });
  const response = await proxyMedia(new Request("https://caper.chat/api/media/status"));
  assert.deepEqual(await response.json(), { enabled: false });
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.equal((await proxyMedia(new Request("https://caper.chat/api/media/secrets"))).status, 404);
  assert.equal((await proxyMedia(new Request("https://caper.chat/api/media/join"))).status, 405);
  assert.equal((await proxyMedia(new Request("https://caper.chat/api/media/join", { method: "POST", headers: { "sec-fetch-site": "cross-site" } }))).status, 403);
});

test("adapter forwards only credentials needed by the fixed API and preserves 204", async (t) => {
  const old = process.env.MEDIA_API_URL;
  process.env.MEDIA_API_URL = "http://media:3001";
  t.after(() => { if (old) process.env.MEDIA_API_URL = old; else delete process.env.MEDIA_API_URL; });
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "http://media:3001/api/media/leave");
    assert.equal(new Headers(init.headers).get("authorization"), "Bearer ephemeral");
    assert.equal(new Headers(init.headers).get("cookie"), null);
    return new Response(null, { status: 204 });
  });
  const response = await proxyMedia(new Request("https://caper.chat/api/media/leave", {
    method: "POST", headers: { authorization: "Bearer ephemeral", "content-type": "application/json", cookie: "unrelated=true" }, body: "{}",
  }));
  assert.equal(response.status, 204);
});
