// Delivery for Caper uploads at the CDN origin (for example cdn.caper.chat).
// The API issues `/{assets|preview}/{assetId}?exp=&sig=` with HMAC-SHA256 over
// "{storageKey}\n{exp}" (apps/api/src/assets.rs). `assets/` maps to `original/`
// in R2; legacy `/original/` URLs remain valid. This Worker checks the signature
// and streams the private R2 object. It never lists or writes.

const KEY = /^\/(assets|original|preview)\/([A-Za-z0-9]{1,64})$/;
// Same allowlist as `assets::kind`. Anything else downloads, never renders.
const INLINE = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/avif",
  "video/mp4",
  "video/webm",
  "video/quicktime",
  "audio/mpeg",
  "audio/mp4",
  "audio/x-m4a",
  "audio/aac",
  "audio/ogg",
  "audio/wav",
  "audio/x-wav",
  "audio/webm",
  "audio/flac",
]);
// Documents that open in a browser tab, keeping their name. The API checked
// their signature bytes; browser PDF viewers sandbox the document themselves,
// and a page-level CSP sandbox would stop Chrome from showing it at all.
const VIEWABLE = new Set(["application/pdf"]);
// API URLs live 24–48 hours; refuse anything signed further out.
const MAX_LIFETIME = 3 * 24 * 60 * 60;
const MAX_BROWSER_AGE = 24 * 60 * 60;
// Browsers keep a copy no longer than the signed URL itself is valid, so an
// expired link stops loading even from a local cache.
const browserCache = (expires, now) =>
  `private, max-age=${Math.max(0, Math.min(MAX_BROWSER_AGE, expires - now))}, immutable`;
const encoder = new TextEncoder();

function base64url(bytes) {
  let text = "";
  for (const byte of new Uint8Array(bytes)) text += String.fromCharCode(byte);
  return btoa(text).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function decodeBase64url(value) {
  if (!/^[A-Za-z0-9_-]{43}$/.test(value)) return null;
  const binary = atob(value.replaceAll("-", "+").replaceAll("_", "/") + "=");
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

async function signingKey(secret) {
  return crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
    "verify",
  ]);
}

export async function sign(secret, key, expires) {
  const signature = await crypto.subtle.sign("HMAC", await signingKey(secret), encoder.encode(`${key}\n${expires}`));
  return base64url(signature);
}

async function verified(secret, key, expires, signature) {
  const bytes = decodeBase64url(signature ?? "");
  if (!bytes) return false;
  // WebCrypto verify compares in constant time.
  return crypto.subtle.verify("HMAC", await signingKey(secret), bytes, encoder.encode(`${key}\n${expires}`));
}

// GIFs carry their own loop count (NETSCAPE2.0), or none, which plays once.
// Browsers and some players honor it; Caper's GIFs always loop, as the
// native clients play them. Only this extension changes; every other byte,
// and the stored object, stay as uploaded.
const NETSCAPE = [...encoder.encode("NETSCAPE2.0")];
const LOOP_FOREVER = new Uint8Array([0x21, 0xff, 11, ...NETSCAPE, 3, 1, 0, 0, 0]);
// The loop extension sits before the first frame; give up past this much header.
const GIF_HEAD_LIMIT = 256 * 1024;

/** The patched head of a GIF that loops forever, or null to leave it alone (not a GIF, or cut short). */
export function loopingGifHead(head) {
  const text = String.fromCharCode(...head.subarray(0, 6));
  if (head.length < 13 || (text !== "GIF87a" && text !== "GIF89a")) return null;
  const table = head[10] & 0x80 ? 3 << ((head[10] & 7) + 1) : 0;
  let at = 13 + table;
  while (at < head.length) {
    if (head[at] === 0x2c || head[at] === 0x3b) {
      // The first frame (or the end) with no loop extension: add one after the colour table.
      const patched = new Uint8Array(head.length + LOOP_FOREVER.length);
      patched.set(head.subarray(0, 13 + table));
      patched.set(LOOP_FOREVER, 13 + table);
      patched.set(head.subarray(13 + table), 13 + table + LOOP_FOREVER.length);
      // GIF87a has no extensions.
      patched.set(encoder.encode("GIF89a"));
      return patched;
    }
    if (head[at] !== 0x21 || at + 2 >= head.length) return null;
    if (
      head[at + 1] === 0xff &&
      head[at + 2] === 11 &&
      NETSCAPE.every((byte, i) => head[at + 3 + i] === byte) &&
      head[at + 14] === 3 &&
      head[at + 15] === 1 &&
      at + 17 < head.length
    ) {
      const patched = head.slice();
      patched[at + 16] = 0;
      patched[at + 17] = 0;
      return patched;
    }
    // Skip the extension's sub-blocks.
    at += 2;
    while (at < head.length && head[at] !== 0) at += head[at] + 1;
    at += 1;
  }
  return null;
}

/** `body` (a GIF of `size` bytes) set to loop forever: its new body and size. */
async function loopingGif(body, size) {
  const reader = new Response(body).body.getReader();
  const chunks = [];
  let read = 0;
  let head = new Uint8Array();
  let patched = null;
  let done = false;
  while (!done && read < GIF_HEAD_LIMIT) {
    const next = await reader.read();
    done = next.done;
    if (next.value) {
      chunks.push(next.value);
      read += next.value.length;
    }
    head = new Uint8Array(read);
    let offset = 0;
    for (const chunk of chunks) head.set(chunk, (offset += chunk.length) - chunk.length);
    patched = loopingGifHead(head);
    if (patched) break;
  }
  const first = patched ?? head;
  const length = size + first.length - head.length;
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(first);
    },
    async pull(controller) {
      const next = await reader.read();
      if (next.done) controller.close();
      else controller.enqueue(next.value);
    },
    cancel(reason) {
      return reader.cancel(reason);
    },
  });
  if (typeof FixedLengthStream === "undefined") return { body: stream, size: length };
  // Workers keep the Content-Length of a stream only through a FixedLengthStream.
  const fixed = new FixedLengthStream(length);
  stream.pipeTo(fixed.writable).catch(() => {});
  return { body: fixed.readable, size: length };
}

function deny(status) {
  return new Response(null, { status, headers: { "cache-control": "no-store" } });
}

function headersFor(object, cacheControl) {
  const headers = new Headers();
  object.writeHttpMetadata(headers);
  const type = (headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
  if (INLINE.has(type)) {
    headers.set("content-disposition", "inline");
  } else if (VIEWABLE.has(type)) {
    headers.set(
      "content-disposition",
      (headers.get("content-disposition") ?? "").replace(/^attachment\b/, "inline") || "inline",
    );
  } else {
    headers.set("content-type", "application/octet-stream");
    if (!headers.get("content-disposition")?.startsWith("attachment")) headers.set("content-disposition", "attachment");
  }
  headers.delete("cache-control");
  headers.delete("content-encoding");
  headers.set("etag", object.httpEtag);
  headers.set("accept-ranges", "bytes");
  headers.set("x-content-type-options", "nosniff");
  if (!VIEWABLE.has(type)) headers.set("content-security-policy", "default-src 'none'; sandbox");
  headers.set("cross-origin-resource-policy", "cross-origin");
  // Signed GETs can be read by clients to render a device-only notification image.
  headers.set("access-control-allow-origin", "*");
  // Objects never change once uploaded; the signed URL itself rotates daily.
  headers.set("cache-control", cacheControl);
  return headers;
}

export default {
  async fetch(request, env, ctx) {
    if (request.method !== "GET" && request.method !== "HEAD") return deny(405);
    const url = new URL(request.url);
    const match = KEY.exec(url.pathname);
    const expires = Number(url.searchParams.get("exp"));
    const now = Math.floor(Date.now() / 1000);
    if (!match || !Number.isSafeInteger(expires) || expires <= now || expires > now + MAX_LIFETIME) return deny(404);
    const key = `${match[1] === "assets" ? "original" : match[1]}/${match[2]}`;
    if (!(await verified(env.ASSET_CDN_SIGNING_SECRET, key, expires, url.searchParams.get("sig")))) return deny(404);

    // Authorization is the signature above; the edge cache is keyed by object
    // only, so every valid URL for the same object shares one cached copy.
    const ranged = request.headers.has("range");
    const cacheKey = new Request(`${url.origin}/${key}`);
    const cache = globalThis.caches?.default;
    if (!ranged && request.method === "GET" && cache) {
      const hit = await cache.match(cacheKey);
      // A GIF cached before it was set to loop is fetched again.
      if (hit && (hit.headers.get("content-type") !== "image/gif" || hit.headers.has("x-caper-gif-loop"))) {
        const response = new Response(hit.body, hit);
        response.headers.set("cache-control", browserCache(expires, now));
        response.headers.set("access-control-allow-origin", "*");
        return response;
      }
    }

    let object;
    try {
      object =
        request.method === "HEAD"
          ? await env.MEDIA.head(key)
          : await env.MEDIA.get(key, { range: request.headers, onlyIf: request.headers });
    } catch (error) {
      // R2 throws for a range past the end, such as resuming a download that
      // already finished. Say so, or the client retries a 500 forever.
      if (!ranged || !String(error?.message).includes("(10039)")) throw error;
      const whole = await env.MEDIA.head(key);
      if (!whole) return deny(404);
      return new Response(null, {
        status: 416,
        headers: { "cache-control": "no-store", "content-range": `bytes */${whole.size}` },
      });
    }
    if (!object) return deny(404);
    const headers = headersFor(object, browserCache(expires, now));
    const gif = headers.get("content-type") === "image/gif";
    if (gif) {
      // The served bytes differ from the stored object's, so their tag does too.
      headers.set("etag", `W/${object.httpEtag}`);
      headers.set("x-caper-gif-loop", "1");
    }
    if (!("body" in object) || object.body == null) {
      // HEAD, or a conditional request that matched (If-None-Match).
      // A GIF's served size is known only once its head is read.
      if (!gif) headers.set("content-length", String(object.size));
      return new Response(null, { status: request.method === "HEAD" ? 200 : 304, headers });
    }
    let body = object.body;
    let size = object.size;
    if (gif) {
      // Whole, even when a range was asked for (servers may ignore Range):
      // offsets into the stored GIF don't match the served one.
      if (object.range) {
        await object.body.cancel?.();
        body = (await env.MEDIA.get(key))?.body;
      }
      if (!body) return deny(404);
      ({ body, size } = await loopingGif(body, object.size));
    } else if (ranged && object.range) {
      // R2Range is {offset, length?}, {offset?, length} or a suffix (`bytes=-N`).
      const { suffix } = object.range;
      const offset = object.range.offset ?? (suffix === undefined ? 0 : object.size - Math.min(suffix, object.size));
      const length = object.range.length ?? object.size - offset;
      headers.set("content-range", `bytes ${offset}-${offset + length - 1}/${object.size}`);
      headers.set("content-length", String(length));
      return new Response(object.body, { status: 206, headers });
    }
    headers.set("content-length", String(size));
    const response = new Response(body, { status: 200, headers });
    if (cache && request.method === "GET") {
      const cached = response.clone();
      const shared = new Response(cached.body, cached);
      shared.headers.set("cache-control", "public, max-age=604800, immutable");
      ctx?.waitUntil?.(cache.put(cacheKey, shared));
    }
    return response;
  },
};
