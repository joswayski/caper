import assert from "node:assert/strict";
import { test } from "node:test";
import { attachmentsOf, isChatMessage, type ChatMessage } from "../chat/types.ts";
import { attachmentKind, compressible, uploadsAvailable, fitWithin, formatBytes, keepCompressed, refreshAttachmentUrls, renamed, uploadPrepared, urlExpiry, UploadError, type UploadTransport } from "../chat/uploads.ts";

const message = (content: unknown) => ({
  id: "m1", channelId: "c1", seq: "1", createdAt: "2026-10-01T00:00:00Z", clientMessageId: "x",
  author: { id: "a", name: "A", isGuest: false }, content,
});

test("messages with attachments stay valid and malformed files are skipped, not fatal", () => {
  const value = message({ version: 1, type: "text", text: "", attachments: [
    { id: "f1", kind: "image", contentType: "image/webp", name: "shot.webp", size: 10, width: 4, height: 3, url: "https://cdn.test/original/f1?exp=1&sig=s" },
    { id: "f2", kind: "script", contentType: "text/html", name: "x", size: 1 },
    { id: "f3", kind: "file", contentType: "text/plain", name: "a.txt", size: 2, url: "javascript:alert(1)" },
  ] });
  assert.ok(isChatMessage(value));
  assert.deepEqual(attachmentsOf(value as ChatMessage).map((item) => item.id), ["f1"]);
  assert.ok(isChatMessage(message({ version: 1, type: "text", text: "plain" })));
  assert.ok(!isChatMessage(message({ version: 1, type: "text", text: "", attachments: "nope" })));
});

test("inline kinds match the API allowlist and SVG never renders inline", () => {
  assert.equal(attachmentKind("image/png"), "image");
  assert.equal(attachmentKind("video/quicktime"), "video");
  assert.equal(attachmentKind("audio/mpeg"), "audio");
  assert.equal(attachmentKind("image/svg+xml"), "file");
  assert.equal(attachmentKind("image/heic"), "file");
  assert.ok(compressible("image/png") && compressible("image/heic"));
  assert.ok(!compressible("image/gif") && !compressible("image/svg+xml"));
});

test("compression is kept only when it saves space or makes the file viewable", () => {
  assert.ok(keepCompressed({ type: "image/png", size: 1000 }, { size: 280 }));
  assert.ok(!keepCompressed({ type: "image/jpeg", size: 1000 }, { size: 950 }));
  assert.ok(keepCompressed({ type: "image/heic", size: 1000 }, { size: 1200 }));
  assert.equal(renamed("Screenshot 2026.png", "image/webp"), "Screenshot 2026.webp");
  assert.equal(renamed("noext", "image/jpeg"), "noext.jpg");
  assert.deepEqual(fitWithin(3840, 2160), { width: 640, height: 360 });
  assert.deepEqual(fitWithin(300, 200), { width: 300, height: 200 });
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(18.4 * 1024 * 1024), "18 MB");
  assert.equal(formatBytes(1.5 * 1024 * 1024), "1.5 MB");
});

test("uploads reserve, put the preview and original with the signed headers, then confirm", async () => {
  const calls: string[] = [];
  const puts: Array<{ url: string; headers: Record<string, string>; size: number }> = [];
  const progress: number[] = [];
  const transport: UploadTransport = {
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method} ${String(input)}`);
      if (String(input) === "/api/assets") {
        const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
        assert.equal(body.channelId, "c1");
        assert.equal(body.byteSize, 300);
        assert.equal(body.sourceByteSize, 1200);
        assert.deepEqual(body.preview, { contentType: "image/webp", byteSize: 100 });
        return Response.json({
          id: "f1",
          upload: { method: "PUT", url: "https://r2.test/original", headers: { "content-type": "image/webp", "content-disposition": "attachment" } },
          previewUpload: { method: "PUT", url: "https://r2.test/preview", headers: { "content-type": "image/webp" } },
        }, { status: 201 });
      }
      return Response.json({ id: "f1", kind: "image", contentType: "image/webp", name: "a.webp", size: 300, preview: {} });
    }) as typeof fetch,
    put: async (url, headers, body, report) => { puts.push({ url, headers, size: body.size }); report(1); },
  };
  const attachment = await uploadPrepared("c1", {
    blob: new Blob([new Uint8Array(300)], { type: "image/webp" }), name: "a.webp", contentType: "image/webp", kind: "image",
    sourceSize: 1200, width: 40, height: 30, preview: new Blob([new Uint8Array(100)], { type: "image/webp" }),
  }, transport, (value) => progress.push(value), new AbortController().signal);
  assert.equal(attachment.id, "f1");
  assert.deepEqual(calls, ["POST /api/assets", "POST /api/assets/f1/complete"]);
  assert.deepEqual(puts.map((put) => [put.url, put.size]), [["https://r2.test/preview", 100], ["https://r2.test/original", 300]]);
  assert.equal(puts[1].headers["content-disposition"], "attachment");
  assert.equal(progress.at(-1), 1);
});

test("a full storage allowance surfaces a clear, typed error", async () => {
  const transport: UploadTransport = {
    fetch: (async () => Response.json({ error: "storage limit reached", code: "storage_full" }, { status: 413 })) as typeof fetch,
    put: async () => { throw new Error("must not upload"); },
  };
  await assert.rejects(
    uploadPrepared("c1", { blob: new Blob(["x"]), name: "x", contentType: "", kind: "file", sourceSize: 1 }, transport, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && error.storageFull,
  );
});

test("expired URLs can be refreshed and their expiry read", async () => {
  const urls = await refreshAttachmentUrls(["f1"], (async (_: unknown, init?: RequestInit) => {
    assert.deepEqual(JSON.parse(String(init?.body)), { ids: ["f1"] });
    return Response.json({ urls: { f1: { url: "https://cdn.test/original/f1?exp=172800&sig=s" } } });
  }) as typeof fetch);
  assert.equal(urlExpiry(urls.f1.url), 172800);
  assert.equal(urlExpiry(undefined), undefined);
  assert.deepEqual(await refreshAttachmentUrls(["f1"], (async () => new Response(null, { status: 503 })) as typeof fetch), {});
});

test("the attach control appears only when the API has uploads configured", async () => {
  assert.equal(await uploadsAvailable((async () => Response.json({ used: 0, limit: 1 })) as typeof fetch), true);
  assert.equal(await uploadsAvailable((async () => Response.json({ error: "uploads unavailable" }, { status: 503 })) as typeof fetch), false);
  assert.equal(await uploadsAvailable((async () => { throw new TypeError("offline"); }) as typeof fetch), false);
});
