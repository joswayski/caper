import assert from "node:assert/strict";
import test from "node:test";
import { proxyMedia as forwardMedia } from "./media.ts";

const proxyMedia = (request: Request) => forwardMedia(request, "verified-account-fixture");

test("adapter fails closed without an account token", async () => {
  assert.equal((await forwardMedia(new Request("https://caper.chat/api/media/status"), "")).status, 401);
});

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
    assert.equal(new Headers(init.headers).get("authorization"), "Bearer verified-account-fixture");
    assert.equal(new Headers(init.headers).get("x-caper-media-token"), "ephemeral");
    assert.equal(new Headers(init.headers).get("x-caper-account-token"), null);
    assert.equal(new Headers(init.headers).get("cookie"), null);
    return new Response(null, { status: 204 });
  });
  const response = await proxyMedia(new Request("https://caper.chat/api/media/leave", {
    method: "POST", headers: { authorization: "Bearer ephemeral", "content-type": "application/json", cookie: "unrelated=true", "x-caper-account-token": "attacker-supplied" }, body: "{}",
  }));
  assert.equal(response.status, 204);
});

test("adapter preserves the Rust failure reference without exposing other upstream headers", async (t) => {
  const old = process.env.MEDIA_API_URL;
  process.env.MEDIA_API_URL = "http://media:3001";
  t.after(() => { if (old) process.env.MEDIA_API_URL = old; else delete process.env.MEDIA_API_URL; });
  const errorId = "01900000-0000-4000-8000-000000000001";
  t.mock.method(globalThis, "fetch", async () => Response.json({ error: "media provider unavailable" }, {
    status: 502, headers: { "x-caper-error-id": errorId, "set-cookie": "secret=value" },
  }));
  const response = await proxyMedia(new Request("https://caper.chat/api/media/join", {
    method: "POST", headers: { "content-type": "application/json" }, body: "{}",
  }));
  assert.equal(response.status, 502);
  assert.equal(response.headers.get("x-caper-error-id"), errorId);
  assert.equal(response.headers.get("set-cookie"), null);
  assert.deepEqual(await response.json(), { error: "media provider unavailable" });
});

test("SSE proxy streams immediately, survives the ordinary deadline, and forwards cancellation", async (t) => {
  const old = process.env.MEDIA_API_URL;
  process.env.MEDIA_API_URL = "http://media:3001";
  t.after(() => { if (old) process.env.MEDIA_API_URL = old; else delete process.env.MEDIA_API_URL; });
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let signal!: AbortSignal;
  let stream!: ReadableStreamDefaultController<Uint8Array>;
  let cancelled = false;
  t.mock.method(globalThis, "fetch", async (url: string, init: RequestInit) => {
    assert.equal(url, "http://media:3001/api/media/events");
    assert.equal(init.method, "GET");
    assert.equal(new Headers(init.headers).get("authorization"), "Bearer verified-account-fixture");
    assert.equal(new Headers(init.headers).get("x-caper-media-token"), "ephemeral");
    signal = init.signal!;
    return new Response(new ReadableStream<Uint8Array>({
      start(controller) { stream = controller; controller.enqueue(new TextEncoder().encode("event: ready\ndata: {}\n\n")); },
      cancel() { cancelled = true; },
    }), { headers: { "content-type": "text/event-stream" } });
  });
  const controller = new AbortController();
  const response = await proxyMedia(new Request("https://caper.chat/api/media/events", {
    headers: { authorization: "Bearer ephemeral" }, signal: controller.signal,
  }));
  assert.equal(response.headers.get("x-accel-buffering"), "no");
  assert.equal(response.headers.get("content-type"), "text/event-stream");
  const reader = response.body!.getReader();
  assert.match(new TextDecoder().decode((await reader.read()).value), /event: ready/);
  t.mock.timers.tick(26_000);
  assert.equal(signal.aborted, false, "the streaming body must outlive the normal request timeout");
  stream.enqueue(new TextEncoder().encode("event: heartbeat\ndata: {}\n\n"));
  assert.match(new TextDecoder().decode((await reader.read()).value), /heartbeat/);
  controller.abort();
  assert.equal(signal.aborted, true);
  await reader.cancel();
  assert.equal(cancelled, true);
});
