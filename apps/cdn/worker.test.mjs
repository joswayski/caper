import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { gunzipSync, gzipSync } from "node:zlib";
import { test } from "node:test";
import worker, { sign } from "./worker.mjs";

const SECRET = "0123456789abcdef0123456789abcdef";

// Fake R2 binding with the subset of the Workers API the Worker uses.
function bucket(objects) {
  const metadata = (key) => ({
    size: objects[key].body.length,
    httpEtag: `"${key}"`,
    writeHttpMetadata(headers) {
      for (const [name, value] of Object.entries(objects[key].headers)) headers.set(name, value);
    },
  });
  return {
    async head(key) {
      return objects[key] ? metadata(key) : null;
    },
    async get(key, { range } = {}) {
      if (!objects[key]) return null;
      const body = objects[key].body;
      const header = range?.get?.("range");
      const match = header && /^bytes=(\d+)-(\d+)$/.exec(header);
      if (match) {
        const offset = Number(match[1]);
        const length = Number(match[2]) - offset + 1;
        return { ...metadata(key), range: { offset, length }, body: body.slice(offset, offset + length) };
      }
      return { ...metadata(key), body };
    },
  };
}

const env = {
  ASSET_CDN_SIGNING_SECRET: SECRET,
  MEDIA: bucket({
    "original/abc": { body: new TextEncoder().encode("0123456789"), headers: { "content-type": "image/png", "content-disposition": "attachment; filename=\"a.png\"" } },
    "original/doc": { body: new TextEncoder().encode("<html>"), headers: { "content-type": "text/html", "content-disposition": "attachment; filename=\"page.html\"" } },
    "original/notes": { body: gzipSync("hello hello hello hello"), headers: { "content-type": "text/markdown", "content-disposition": "attachment; filename=\"notes.md\"", "content-encoding": "gzip" } },
  }),
};

async function signed(key, expires = Math.floor(Date.now() / 1000) + 3600) {
  return `https://cdn.test/${key}?exp=${expires}&sig=${await sign(SECRET, key, expires)}`;
}

test("signatures match the API's HMAC-SHA256 of key and expiry", async () => {
  const expected = createHmac("sha256", SECRET).update("original/abc\n172800").digest("base64url");
  // Same vector as apps/api/src/assets/tests.rs.
  assert.equal(expected, "2GDNk1-0tzBzLEfmuQIvaGGFQ3ItbeJMj8gWdbnL8Ck");
  assert.equal(await sign(SECRET, "original/abc", 172800), expected);
});

test("valid URLs stream allowlisted media inline", async () => {
  const response = await worker.fetch(new Request(await signed("original/abc")), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "image/png");
  assert.equal(response.headers.get("content-disposition"), "inline");
  assert.equal(response.headers.get("x-content-type-options"), "nosniff");
  assert.equal(response.headers.get("cache-control"), "private, max-age=86400, immutable");
  assert.equal(await response.text(), "0123456789");
});

test("other types download as opaque bytes with their name", async () => {
  const response = await worker.fetch(new Request(await signed("original/doc")), env);
  assert.equal(response.headers.get("content-type"), "application/octet-stream");
  assert.equal(response.headers.get("content-disposition"), "attachment; filename=\"page.html\"");
});

test("ranges return partial content for video seeking", async () => {
  const response = await worker.fetch(new Request(await signed("original/abc"), { headers: { range: "bytes=2-5" } }), env);
  assert.equal(response.status, 206);
  assert.equal(response.headers.get("content-range"), "bytes 2-5/10");
  assert.equal(await response.text(), "2345");
});

test("tampered, expired, overlong, foreign-key and unknown requests are refused", async () => {
  const now = Math.floor(Date.now() / 1000);
  const good = new URL(await signed("original/abc"));
  const tampered = new URL(good);
  tampered.searchParams.set("exp", String(now + 7200));
  const otherKey = new URL(good);
  otherKey.pathname = "/preview/abc";
  for (const url of [
    tampered,
    otherKey,
    await signed("original/abc", now - 1),
    await signed("original/abc", now + 4 * 24 * 3600),
    "https://cdn.test/original/abc",
    "https://cdn.test/other/abc?exp=1&sig=x",
    await signed("original/missing"),
  ]) {
    assert.equal((await worker.fetch(new Request(url), env)).status, 404, String(url));
  }
  assert.equal((await worker.fetch(new Request(good, { method: "PUT", body: "x" }), env)).status, 405);
});

test("HEAD reports size without a body", async () => {
  const response = await worker.fetch(new Request(await signed("original/abc"), { method: "HEAD" }), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-length"), "10");
});

test("gzip-stored documents pass through to clients that accept gzip", async () => {
  const response = await worker.fetch(new Request(await signed("original/notes"), { headers: { "accept-encoding": "br, gzip;q=0.8", range: "bytes=0-3" } }), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-encoding"), "gzip");
  assert.equal(response.headers.get("vary"), "accept-encoding");
  assert.equal(response.headers.get("content-type"), "application/octet-stream");
  assert.equal(response.headers.get("accept-ranges"), null);
  assert.equal(gunzipSync(Buffer.from(await response.arrayBuffer())).toString(), "hello hello hello hello");
});

test("gzip-stored documents are decompressed for clients that do not accept gzip", async () => {
  for (const accept of [undefined, "identity", "gzip;q=0"]) {
    const headers = accept ? { "accept-encoding": accept } : {};
    const response = await worker.fetch(new Request(await signed("original/notes"), { headers }), env);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("content-encoding"), null);
    assert.equal(response.headers.get("content-length"), null);
    assert.equal(await response.text(), "hello hello hello hello");
  }
});
