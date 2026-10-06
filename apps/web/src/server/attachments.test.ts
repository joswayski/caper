import assert from "node:assert/strict";
import { test } from "node:test";
import { attachmentView, attachmentsOf, isChatAttachment, isChatAttachmentProgressEvent, isChatAttachmentsEvent, isChatMessage, type ChatAttachment, type ChatMessage } from "../chat/types.ts";
import { formatBytes, refreshAttachmentUrls, uploadFile, uploadSettings, uploadSizeError, urlExpiry, UploadError, type UploadTransport } from "../chat/uploads.ts";

const message = (content: unknown) => ({
  id: "m1", channelId: "c1", seq: "1", createdAt: "2026-10-01T00:00:00Z", clientMessageId: "x",
  author: { id: "a", name: "A", isGuest: false }, content,
});

const file = (overrides: Partial<ChatAttachment> = {}): ChatAttachment => ({
  id: "f1", kind: "image", contentType: "image/avif", name: "a.avif", size: 10, ...overrides,
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
  assert.ok(isChatMessage({ ...message({ version: 1, type: "text", text: "" }), attachmentsSeq: "7" }));
  assert.ok(!isChatMessage({ ...message({ version: 1, type: "text", text: "" }), attachmentsSeq: "07" }));
});

test("attachment validation accepts processing states and animation, tolerates extra fields, rejects garbage", () => {
  for (const status of [undefined, "processing", "ready", "failed"] as const) assert.ok(isChatAttachment(file({ status })), String(status));
  assert.ok(isChatAttachment({ ...file(), status: "processing", previewUrl: "https://cdn.test/preview/f1", preview: {} }));
  assert.ok(isChatAttachment({ ...file({ kind: "video", contentType: "video/mp4" }), animated: true, futureField: { nested: 1 } }));
  assert.ok(!isChatAttachment({ ...file(), status: "done" }));
  assert.ok(!isChatAttachment({ ...file(), status: 1 }));
  assert.ok(!isChatAttachment({ ...file(), animated: "yes" }));
  assert.ok(!isChatAttachment({ ...file(), previewUrl: "data:image/png;base64,AAAA" }));
  assert.ok(!isChatAttachment({ ...file(), width: -1 }));
});

test("render decision follows status, and animated videos play like GIFs", () => {
  const url = "https://cdn.test/original/f1";
  assert.equal(attachmentView(file({ status: "processing", previewUrl: url })), "processing");
  assert.equal(attachmentView(file({ status: "failed" })), "failed");
  assert.equal(attachmentView(file({ url })), "image", "absent status means ready");
  assert.equal(attachmentView(file({ status: "ready", url })), "image");
  assert.equal(attachmentView(file({ status: "ready", kind: "video", url })), "video");
  assert.equal(attachmentView(file({ status: "ready", kind: "video", animated: true, url })), "animated");
  assert.equal(attachmentView(file({ status: "ready", kind: "video", animated: false, url })), "video");
  assert.equal(attachmentView(file({ status: "ready", kind: "audio", url })), "audio");
  assert.equal(attachmentView(file({ status: "ready", kind: "image" })), "file", "no URL falls back to a file card");
  assert.equal(attachmentView(file({ status: "processing", unavailable: true })), "unavailable");
});

test("live attachment events are validated and unknown fields tolerated", () => {
  const event = { type: "message.attachments", schemaVersion: 1, channelId: "c1", seq: "9", messageId: "m1", attachments: [file()], extra: true };
  assert.ok(isChatAttachmentsEvent(event));
  assert.ok(!isChatAttachmentsEvent({ ...event, seq: "x" }));
  assert.ok(!isChatAttachmentsEvent({ ...event, schemaVersion: 2 }));
  assert.ok(!isChatAttachmentsEvent({ ...event, attachments: {} }));
  const progress = { type: "attachment.progress", channelId: "c1", messageId: "m1", attachmentId: "f1", percent: 42, extra: 1 };
  assert.ok(isChatAttachmentProgressEvent(progress));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, percent: 101 }));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, percent: Number.NaN }));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, attachmentId: 3 }));
});

function transport(complete: Array<() => Response>) {
  const calls: string[] = [];
  const reserved: Array<Record<string, unknown>> = [];
  const puts: Array<{ url: string; headers: Record<string, string>; body: Blob }> = [];
  const sleeps: number[] = [];
  const value: UploadTransport = {
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method} ${String(input)}`);
      if (String(input) === "/api/assets") {
        reserved.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
        return Response.json({
          id: "f1", kind: "image",
          upload: { method: "PUT", url: "https://incoming.s3.test/incoming/f1?X-Amz-Signature=s", headers: { "content-type": "image/png" } },
          storage: { used: 0, limit: 10 },
        }, { status: 201 });
      }
      const next = complete.shift();
      assert.ok(next, "unexpected extra /complete call");
      return next();
    }) as typeof fetch,
    put: async (url, headers, body, report) => { puts.push({ url, headers, body }); report(0.5); report(1); },
    sleep: async (ms) => { sleeps.push(ms); },
  };
  return { value, calls, reserved, puts, sleeps };
}

const processing = () => Response.json({ id: "f1", kind: "image", contentType: "image/png", name: "shot.png", size: 1200, status: "processing" });

test("uploads reserve the original's exact size and type, PUT it unchanged with exactly the signed headers, then confirm", async () => {
  const t = transport([processing]);
  const original = new File([new Uint8Array(1200)], "shot.png", { type: "image/png" });
  const progress: number[] = [];
  const attachment = await uploadFile("c1", original, { maxUploadBytes: 2 ** 31 }, t.value, (value) => progress.push(value), new AbortController().signal);
  assert.equal(attachment.status, "processing");
  assert.deepEqual(t.reserved, [{ channelId: "c1", filename: "shot.png", contentType: "image/png", byteSize: 1200 }], "no compression metadata or previews are reserved");
  assert.deepEqual(t.calls, ["POST /api/assets", "POST /api/assets/f1/complete"]);
  assert.equal(t.puts.length, 1);
  assert.equal(t.puts[0].url, "https://incoming.s3.test/incoming/f1?X-Amz-Signature=s");
  assert.deepEqual(t.puts[0].headers, { "content-type": "image/png" });
  assert.equal(t.puts[0].body, original, "the original bytes go up untouched");
  assert.deepEqual(progress, [0.5, 1, 1]);
});

test("files without a browser type are declared as application/octet-stream", async () => {
  const t = transport([processing]);
  await uploadFile("c1", new File(["abc"], "notes"), {}, t.value, () => undefined, new AbortController().signal);
  assert.equal(t.reserved[0].contentType, "application/octet-stream");
  assert.equal(t.reserved[0].byteSize, 3);
});

test("complete retries 409 with short backoff until the upload is visible", async () => {
  const notYet = () => Response.json({ error: "upload not received" }, { status: 409 });
  const t = transport([notYet, notYet, processing]);
  const attachment = await uploadFile("c1", new File(["x"], "a.txt", { type: "text/plain" }), {}, t.value, () => undefined, new AbortController().signal);
  assert.equal(attachment.id, "f1");
  assert.deepEqual(t.sleeps, [250, 500]);
  assert.equal(t.calls.filter((call) => call.endsWith("/complete")).length, 3);

  const stuck = transport(Array.from({ length: 5 }, () => notYet));
  await assert.rejects(uploadFile("c1", new File(["x"], "a.txt"), {}, stuck.value, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && error.message === "upload not received");
  assert.equal(stuck.sleeps.length, 4, "gives up after a few attempts");

  const mismatch = transport([() => Response.json({ error: "size mismatch" }, { status: 422 })]);
  await assert.rejects(uploadFile("c1", new File(["x"], "a.txt"), {}, mismatch.value, () => undefined, new AbortController().signal),
    /did not arrive intact/);
  assert.deepEqual(mismatch.sleeps, [], "422 is final");
});

test("files over the server's upload limit fail before reserving", async () => {
  const t = transport([]);
  await assert.rejects(uploadFile("c1", new File([new Uint8Array(11)], "big.mov"), { maxUploadBytes: 10 }, t.value, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && /larger than the 10 B upload limit/.test(error.message));
  assert.deepEqual(t.calls, []);
  assert.equal(uploadSizeError(2 * 1024 ** 3, { maxUploadBytes: 2 * 1024 ** 3 }), undefined);
  assert.equal(uploadSizeError(2 * 1024 ** 3 + 1, { maxUploadBytes: 2 * 1024 ** 3 }), "This file is larger than the 2.0 GB upload limit.");
  assert.equal(uploadSizeError(10 ** 12, {}), undefined, "older servers enforce the limit themselves");
});

test("a full storage allowance surfaces a clear, typed error", async () => {
  const value: UploadTransport = {
    fetch: (async () => Response.json({ error: "storage limit reached", code: "storage_full" }, { status: 413 })) as typeof fetch,
    put: async () => { throw new Error("must not upload"); },
  };
  await assert.rejects(
    uploadFile("c1", new File(["x"], "x"), {}, value, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && error.storageFull,
  );
});

test("expired URLs can be refreshed, garbage is dropped, and expiry is readable", async () => {
  const urls = await refreshAttachmentUrls(["f1", "f2", "f3"], (async (_: unknown, init?: RequestInit) => {
    assert.deepEqual(JSON.parse(String(init?.body)), { ids: ["f1", "f2", "f3"] });
    return Response.json({ urls: {
      f1: { url: "https://cdn.test/original/f1?exp=172800&sig=s", previewUrl: "https://cdn.test/preview/f1?exp=172800&sig=s" },
      f2: { previewUrl: "https://cdn.test/preview/f2" },
      f3: { url: "javascript:alert(1)" },
    } });
  }) as typeof fetch);
  assert.equal(urlExpiry(urls.f1.url), 172800);
  assert.deepEqual(urls.f2, { previewUrl: "https://cdn.test/preview/f2" }, "processing files have no url yet");
  assert.equal(urls.f3, undefined);
  assert.equal(urlExpiry(undefined), undefined);
  assert.deepEqual(await refreshAttachmentUrls(["f1"], (async () => new Response(null, { status: 503 })) as typeof fetch), {});
});

test("the attach control appears only when the API has uploads configured, with its upload limit", async () => {
  assert.deepEqual(await uploadSettings((async () => Response.json({ used: 0, limit: 1, maxUploadBytes: 2147483648 })) as typeof fetch), { maxUploadBytes: 2147483648 });
  assert.deepEqual(await uploadSettings((async () => Response.json({ used: 0, limit: 1, compression: { imageQuality: 1 } })) as typeof fetch), {}, "older servers still enable uploads");
  assert.deepEqual(await uploadSettings((async () => Response.json({ used: 0, limit: 1, maxUploadBytes: "big" })) as typeof fetch), {});
  assert.equal(await uploadSettings((async () => Response.json({ error: "uploads unavailable" }, { status: 503 })) as typeof fetch), undefined);
  assert.equal(await uploadSettings((async () => { throw new TypeError("offline"); }) as typeof fetch), undefined);
});

test("byte sizes format compactly", () => {
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(18.4 * 1024 * 1024), "18 MB");
  assert.equal(formatBytes(1.5 * 1024 * 1024), "1.5 MB");
});
