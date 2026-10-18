import assert from "node:assert/strict";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { DraftAttachments, MessageAttachments } from "../chat/Attachments.tsx";
import {
  attachmentView,
  attachmentsOf,
  isChatAttachment,
  isChatAttachmentProgressEvent,
  isChatAttachmentsEvent,
  isChatMessage,
  type ChatAttachment,
  type ChatMessage,
} from "../chat/types.ts";
import {
  AttachmentUpload,
  formatBytes,
  refreshAttachmentUrls,
  uploadPrepared,
  uploadSettings,
  urlExpiry,
  UploadError,
  type UploadTransport,
} from "../chat/uploads.ts";
import { DEFAULT_COMPRESSION, type PreparedFile } from "../chat/prepare.ts";

const message = (content: unknown) => ({
  id: "m1",
  channelId: "c1",
  seq: "1",
  createdAt: "2026-10-01T00:00:00Z",
  clientMessageId: "x",
  author: { id: "a", name: "A", isGuest: false },
  content,
});

const file = (overrides: Partial<ChatAttachment> = {}): ChatAttachment => ({
  id: "f1",
  kind: "image",
  contentType: "image/avif",
  name: "a.avif",
  size: 10,
  ...overrides,
});

test("messages with attachments stay valid and malformed files are skipped, not fatal", () => {
  const value = message({
    version: 1,
    type: "text",
    text: "",
    attachments: [
      {
        id: "f1",
        kind: "image",
        contentType: "image/webp",
        name: "shot.webp",
        size: 10,
        width: 4,
        height: 3,
        url: "https://cdn.test/original/f1?exp=1&sig=s",
      },
      { id: "f2", kind: "script", contentType: "text/html", name: "x", size: 1 },
      { id: "f3", kind: "file", contentType: "text/plain", name: "a.txt", size: 2, url: "javascript:alert(1)" },
    ],
  });
  assert.ok(isChatMessage(value));
  assert.deepEqual(
    attachmentsOf(value as ChatMessage).map((item) => item.id),
    ["f1"],
  );
  assert.ok(isChatMessage(message({ version: 1, type: "text", text: "plain" })));
  assert.ok(!isChatMessage(message({ version: 1, type: "text", text: "", attachments: "nope" })));
  assert.ok(isChatMessage({ ...message({ version: 1, type: "text", text: "" }), attachmentsSeq: "7" }));
  assert.ok(!isChatMessage({ ...message({ version: 1, type: "text", text: "" }), attachmentsSeq: "07" }));
});

test("attachment validation accepts processing states and animation, tolerates extra fields, rejects garbage", () => {
  for (const status of [undefined, "processing", "ready", "failed"] as const)
    assert.ok(isChatAttachment(file({ status })), String(status));
  assert.ok(
    isChatAttachment({ ...file(), status: "processing", previewUrl: "https://cdn.test/preview/f1", preview: {} }),
  );
  assert.ok(
    isChatAttachment({
      ...file({ kind: "video", contentType: "video/mp4" }),
      animated: true,
      futureField: { nested: 1 },
    }),
  );
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

test("GIFs render their original even with a still preview; photos keep using previews", () => {
  const url = "https://cdn.test/original/gif";
  const previewUrl = "https://cdn.test/preview/gif";
  for (const contentType of ["image/gif", "image/jpeg", "image/avif"]) {
    const markup = renderToStaticMarkup(
      createElement(MessageAttachments, { attachments: [file({ contentType, url, previewUrl })] }),
    );
    const source = markup.match(/<img\b[^>]*\bsrc="([^"]+)"/)?.[1];
    assert.equal(source, contentType === "image/gif" ? url : previewUrl, contentType);
  }
});

test("live attachment events are validated and unknown fields tolerated", () => {
  const event = {
    type: "message.attachments",
    schemaVersion: 1,
    channelId: "c1",
    seq: "9",
    messageId: "m1",
    attachments: [file()],
    extra: true,
  };
  assert.ok(isChatAttachmentsEvent(event));
  assert.ok(!isChatAttachmentsEvent({ ...event, seq: "x" }));
  assert.ok(!isChatAttachmentsEvent({ ...event, schemaVersion: 2 }));
  assert.ok(!isChatAttachmentsEvent({ ...event, attachments: {} }));
  const progress = {
    type: "attachment.progress",
    channelId: "c1",
    messageId: "m1",
    attachmentId: "f1",
    percent: 42,
    extra: 1,
  };
  assert.ok(isChatAttachmentProgressEvent(progress));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, percent: 101 }));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, percent: Number.NaN }));
  assert.ok(!isChatAttachmentProgressEvent({ ...progress, attachmentId: 3 }));
});

function transport(complete: Array<() => Response>, withPreview = false) {
  const calls: string[] = [];
  const reserved: Array<Record<string, unknown>> = [];
  const puts: Array<{ url: string; headers: Record<string, string>; body: Blob }> = [];
  const sleeps: number[] = [];
  const value: UploadTransport = {
    fetch: (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push(`${init?.method} ${String(input)}`);
      if (String(input) === "/api/assets") {
        reserved.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
        return Response.json(
          {
            id: "f1",
            kind: "image",
            upload: {
              method: "PUT",
              url: "https://r2.test/original/f1?X-Amz-Signature=s",
              headers: { "content-type": "image/webp", "content-disposition": 'attachment; filename="shot.webp"' },
            },
            ...(withPreview
              ? {
                  previewUpload: {
                    method: "PUT",
                    url: "https://r2.test/preview/f1?X-Amz-Signature=p",
                    headers: { "content-type": "image/webp" },
                  },
                }
              : {}),
            storage: { used: 0, limit: 10 },
          },
          { status: 201 },
        );
      }
      if (String(input).endsWith("/cancel")) return new Response(null, { status: 204 });
      const next = complete.shift();
      assert.ok(next, "unexpected extra /complete call");
      return next();
    }) as typeof fetch,
    put: async (url, headers, body, report) => {
      calls.push(`PUT ${url.split("?")[0]}`);
      puts.push({ url, headers, body });
      report(0.5);
      report(1);
    },
    sleep: async (ms) => {
      sleeps.push(ms);
    },
  };
  return { value, calls, reserved, puts, sleeps };
}

const ready = () =>
  Response.json({ id: "f1", kind: "image", contentType: "image/webp", name: "shot.webp", size: 300, preview: {} });

const prepared = (overrides: Partial<PreparedFile> = {}): PreparedFile => ({
  blob: new Blob([new Uint8Array(300)], { type: "image/webp" }),
  name: "shot.webp",
  contentType: "image/webp",
  kind: "image",
  sourceSize: 1200,
  ...overrides,
});

test("uploads reserve the compressed size with source size and preview, PUT the preview then the file with exactly the signed headers, then confirm", async () => {
  const t = transport([ready], true);
  const file = prepared({ width: 40, height: 30, preview: new Blob([new Uint8Array(100)], { type: "image/webp" }) });
  const progress: number[] = [];
  const attachment = await uploadPrepared(
    "c1",
    file,
    t.value,
    (value) => progress.push(value),
    new AbortController().signal,
  );
  assert.equal(attachment.id, "f1");
  assert.deepEqual(t.reserved, [
    {
      channelId: "c1",
      filename: "shot.webp",
      contentType: "image/webp",
      byteSize: 300,
      sourceByteSize: 1200,
      width: 40,
      height: 30,
      preview: { contentType: "image/webp", byteSize: 100 },
    },
  ]);
  assert.deepEqual(t.calls, [
    "POST /api/assets",
    "PUT https://r2.test/preview/f1",
    "PUT https://r2.test/original/f1",
    "POST /api/assets/f1/complete",
  ]);
  assert.deepEqual(t.puts[0].headers, { "content-type": "image/webp" });
  assert.deepEqual(t.puts[1].headers, {
    "content-type": "image/webp",
    "content-disposition": 'attachment; filename="shot.webp"',
  });
  assert.equal(t.puts[0].body, file.preview);
  assert.equal(t.puts[1].body, file.blob, "the prepared bytes go up untouched");
  assert.deepEqual(progress, [0.125, 0.25, 0.625, 1, 1], "progress spans preview and file by size");
});

test("removing during reservation, storage PUT or completion cancels the reservation without cancelling cleanup", async () => {
  for (const stage of ["reservation", "put", "complete"]) {
    const controller = new AbortController();
    const t = transport([ready]);
    const original = t.value.fetch;
    t.value.fetch = async (input, init) => {
      const response = await original(input, init);
      if (
        (stage === "reservation" && String(input) === "/api/assets") ||
        (stage === "complete" && String(input).endsWith("/complete"))
      )
        controller.abort();
      return response;
    };
    if (stage === "put")
      t.value.put = async () => {
        controller.abort();
        controller.signal.throwIfAborted();
      };
    await assert.rejects(
      uploadPrepared("c1", prepared(), t.value, () => undefined, controller.signal),
      { name: "AbortError" },
    );
    assert.equal(t.calls.at(-1), "POST /api/assets/f1/cancel");
    if (stage === "reservation") assert.equal(t.puts.length, 0);
  }
});

test("files without a preview or a browser type reserve only what they have", async () => {
  const t = transport([ready]);
  await uploadPrepared(
    "c1",
    { blob: new Blob(["abc"]), name: "notes", contentType: "", kind: "file", sourceSize: 3 },
    t.value,
    () => undefined,
    new AbortController().signal,
  );
  assert.deepEqual(t.reserved[0], {
    channelId: "c1",
    filename: "notes",
    contentType: "application/octet-stream",
    byteSize: 3,
    sourceByteSize: 3,
  });
  assert.equal(t.puts.length, 1);
});

test("retained drafts coalesce focus and Send, renew after 23 hours, and retry the same bytes", async () => {
  let now = 1000;
  const t = transport([ready, ready, () => new Response(null, { status: 503 }), ready]);
  const upload = new AttachmentUpload(
    "c1",
    new File(["local source"], "notes.txt", { type: "text/plain" }),
    DEFAULT_COMPRESSION,
    () => {},
    t.value,
    () => now,
  );
  const first = upload.ensure();
  assert.equal(upload.ensure(), first, "concurrent triggers share one upload");
  await first;
  now += 23 * 60 * 60 * 1000 - 1;
  await upload.ensure();
  assert.equal(t.puts.length, 1, "fresh files are reused");
  now++;
  await Promise.all([upload.ensure(), upload.ensure()]);
  assert.equal(t.puts.length, 2);
  upload.invalidate();
  await assert.rejects(upload.ensure());
  assert.equal(upload.state.error, "This file could not be uploaded.");
  await upload.ensure();
  assert.equal(upload.state.error, undefined);
  assert.deepEqual(await Promise.all(t.puts.map(({ body }) => body.text())), Array(4).fill("local source"));
  const cancelled = t.calls.filter((call) => call.endsWith("/cancel")).length;
  upload.release();
  assert.equal(t.calls.filter((call) => call.endsWith("/cancel")).length, cancelled, "confirming does not cancel");
});

test("an edit cancels the earlier upload and never falls back to the unedited original", async () => {
  const t = transport([ready, ready]);
  const states: Array<string | undefined> = [];
  const upload = new AttachmentUpload(
    "c1",
    new File(["TEST FIXTURE photo"], "photo.jpg", { type: "image/jpeg" }),
    DEFAULT_COMPRESSION,
    (state) => states.push(state.error),
    t.value,
  );
  await upload.ensure();
  assert.equal(t.puts.length, 1);
  // No canvas here, so the crop cannot be applied: the file fails instead of uploading as it was.
  await assert.rejects(upload.setEdit({ rotation: 0, crop: { x: 0, y: 0, width: 0.5, height: 1 }, marks: [] }));
  assert.equal(upload.state.error, "Couldn’t apply your edits.");
  assert.equal(upload.state.attachment, undefined);
  assert.equal(t.puts.length, 1, "nothing more was uploaded");
  assert.deepEqual(
    t.calls.filter((call) => call.endsWith("/cancel")),
    ["POST /api/assets/f1/cancel"],
    "the unedited upload was cancelled",
  );
  assert.ok(upload.edit, "Retry keeps the edit");
  await assert.rejects(upload.ensure());
  assert.equal(t.puts.length, 1);
  // Going back to the original uploads it again.
  await upload.setEdit(undefined);
  assert.equal(upload.state.error, undefined);
  assert.equal(t.puts.length, 2);
  assert.equal(await t.puts[1].body.text(), "TEST FIXTURE photo");
  assert.equal(upload.source?.name, "photo.jpg", "the original stays available to the editor");
});

test("a reserved preview without an upload URL is refused rather than left to fail verification", async () => {
  const t = transport([]);
  await assert.rejects(
    uploadPrepared(
      "c1",
      prepared({ preview: new Blob(["p"], { type: "image/webp" }) }),
      t.value,
      () => undefined,
      new AbortController().signal,
    ),
    /invalid response/,
  );
  assert.equal(t.puts.length, 0);
});

test("complete waits for queued metadata processing, bounds retries, and treats 422 as final", async () => {
  const notYet = () => Response.json({ error: "upload not finished" }, { status: 409 });
  const t = transport([notYet, notYet, notYet, notYet, notYet, ready]);
  const attachment = await uploadPrepared("c1", prepared(), t.value, () => undefined, new AbortController().signal);
  assert.equal(attachment.id, "f1");
  assert.deepEqual(t.sleeps, [250, 500, 1_000, 2_000, 5_000]);
  assert.equal(t.calls.filter((call) => call.endsWith("/complete")).length, 6);

  const stuck = transport(Array.from({ length: 300 }, () => notYet));
  await assert.rejects(
    uploadPrepared("c1", prepared(), stuck.value, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && error.message === "upload not finished",
  );
  const paused = stuck.sleeps.reduce((sum, ms) => sum + ms, 0);
  assert.ok(
    paused >= 20 * 60_000 && paused < 21 * 60_000,
    "bounded pauses plus five-second server waits cover a batch",
  );

  const mismatch = transport([
    () => Response.json({ error: "file does not match its declared size or type" }, { status: 422 }),
  ]);
  await assert.rejects(
    uploadPrepared("c1", prepared(), mismatch.value, () => undefined, new AbortController().signal),
    /did not arrive intact/,
  );
  assert.deepEqual(mismatch.sleeps, [], "422 is final");

  const message = "This media could not be cleaned safely. Try another format.";
  const unsafe = transport([() => Response.json({ error: message, code: "media_metadata" }, { status: 422 })]);
  const upload = new AttachmentUpload(
    "c1",
    new File(["source"], "file.bin"),
    DEFAULT_COMPRESSION,
    () => {},
    unsafe.value,
  );
  await assert.rejects(upload.ensure(), (error: unknown) => error instanceof UploadError && error.message === message);
  assert.equal(upload.state.error, message, "explain the privacy rejection, not a corrupt upload");
  const html = renderToStaticMarkup(createElement(DraftAttachments, { drafts: [{ ...upload.state, key: "f1" }] }));
  assert.ok(
    html.includes(`aria-label="${message}"`),
    "the failed draft exposes its privacy reason to assistive technology",
  );
  assert.deepEqual(unsafe.sleeps, [], "unsafe media is never retried");
});

test("a full storage allowance surfaces a clear, typed error", async () => {
  const value: UploadTransport = {
    fetch: (async () =>
      Response.json({ error: "storage limit reached", code: "storage_full" }, { status: 413 })) as typeof fetch,
    put: async () => {
      throw new Error("must not upload");
    },
  };
  await assert.rejects(
    uploadPrepared("c1", prepared(), value, () => undefined, new AbortController().signal),
    (error: unknown) => error instanceof UploadError && error.storageFull,
  );
});

test("expired URLs can be refreshed, garbage is dropped, and expiry is readable", async () => {
  const urls = await refreshAttachmentUrls(["f1", "f2", "f3"], (async (_: unknown, init?: RequestInit) => {
    assert.deepEqual(JSON.parse(String(init?.body)), { ids: ["f1", "f2", "f3"] });
    return Response.json({
      urls: {
        f1: {
          url: "https://cdn.test/original/f1?exp=172800&sig=s",
          previewUrl: "https://cdn.test/preview/f1?exp=172800&sig=s",
        },
        f2: { previewUrl: "https://cdn.test/preview/f2" },
        f3: { url: "javascript:alert(1)" },
      },
    });
  }) as typeof fetch);
  assert.equal(urlExpiry(urls.f1.url), 172800);
  assert.deepEqual(urls.f2, { previewUrl: "https://cdn.test/preview/f2" }, "processing files have no url yet");
  assert.equal(urls.f3, undefined);
  assert.equal(urlExpiry(undefined), undefined);
  assert.deepEqual(
    await refreshAttachmentUrls(["f1"], (async () => new Response(null, { status: 503 })) as typeof fetch),
    {},
  );
  assert.deepEqual(
    await refreshAttachmentUrls(["f1"], (async () => {
      throw new TypeError("Failed to fetch");
    }) as typeof fetch),
    {},
    "an offline refresh is not an unhandled rejection",
  );
});

test("the attach control appears only when the API has uploads configured, with its compression settings", async () => {
  const tuned = await uploadSettings((async () =>
    Response.json({
      used: 0,
      limit: 1,
      compression: { ...DEFAULT_COMPRESSION, imageQuality: 80, videoMaxHeight: 0 },
    })) as typeof fetch);
  assert.equal(tuned?.imageQuality, 80);
  assert.equal(tuned?.videoMaxHeight, 0);
  assert.deepEqual(
    await uploadSettings((async () => Response.json({ used: 0, limit: 1 })) as typeof fetch),
    { ...DEFAULT_COMPRESSION, imageFormat: "webp" },
    "older servers get defaults and WebP photos",
  );
  assert.equal(
    (
      await uploadSettings((async () =>
        Response.json({ used: 0, limit: 1, compression: DEFAULT_COMPRESSION })) as typeof fetch)
    )?.imageFormat,
    "avif",
  );
  assert.equal(
    await uploadSettings((async () =>
      Response.json({ error: "uploads unavailable" }, { status: 503 })) as typeof fetch),
    undefined,
  );
  assert.equal(
    await uploadSettings((async () => {
      throw new TypeError("offline");
    }) as typeof fetch),
    undefined,
  );
});

test("byte sizes format compactly", () => {
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(18.4 * 1024 * 1024), "18 MB");
  assert.equal(formatBytes(1.5 * 1024 * 1024), "1.5 MB");
});
