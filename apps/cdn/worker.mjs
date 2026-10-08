// Delivery for Caper uploads at the CDN origin (for example cdn.caper.chat).
// The API signs `/{original|preview}/{assetId}?exp=&sig=` with HMAC-SHA256 over
// "{key}\n{exp}" (apps/api/src/assets.rs); this Worker only checks that
// signature and streams the private R2 object. It never lists or writes.

const KEY = /^\/(original|preview)\/([A-Za-z0-9]{1,64})$/;
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
// API URLs live 24–48 hours; refuse anything signed further out.
const MAX_LIFETIME = 3 * 24 * 60 * 60;
const PRIVATE = "private, max-age=86400, immutable";
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

function deny(status) {
  return new Response(null, { status, headers: { "cache-control": "no-store" } });
}

function headersFor(object) {
  const headers = new Headers();
  object.writeHttpMetadata(headers);
  const type = (headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
  if (INLINE.has(type)) {
    headers.set("content-disposition", "inline");
  } else {
    headers.set("content-type", "application/octet-stream");
    if (!headers.get("content-disposition")?.startsWith("attachment")) headers.set("content-disposition", "attachment");
  }
  headers.delete("cache-control");
  headers.delete("content-encoding");
  headers.set("etag", object.httpEtag);
  headers.set("accept-ranges", "bytes");
  headers.set("x-content-type-options", "nosniff");
  headers.set("content-security-policy", "default-src 'none'; sandbox");
  headers.set("cross-origin-resource-policy", "cross-origin");
  // Objects never change once uploaded; the signed URL itself rotates daily.
  headers.set("cache-control", PRIVATE);
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
    const key = `${match[1]}/${match[2]}`;
    if (!(await verified(env.ASSET_CDN_SIGNING_SECRET, key, expires, url.searchParams.get("sig")))) return deny(404);

    // Authorization is the signature above; the edge cache is keyed by object
    // only, so every valid URL for the same object shares one cached copy.
    const ranged = request.headers.has("range");
    const cacheKey = new Request(`${url.origin}/${key}`);
    const cache = globalThis.caches?.default;
    if (!ranged && request.method === "GET" && cache) {
      const hit = await cache.match(cacheKey);
      if (hit) {
        const response = new Response(hit.body, hit);
        response.headers.set("cache-control", PRIVATE);
        return response;
      }
    }

    const object =
      request.method === "HEAD"
        ? await env.MEDIA.head(key)
        : await env.MEDIA.get(key, { range: request.headers, onlyIf: request.headers });
    if (!object) return deny(404);
    const headers = headersFor(object);
    if (!("body" in object) || object.body == null) {
      // HEAD, or a conditional request that matched (If-None-Match).
      headers.set("content-length", String(object.size));
      return new Response(null, { status: request.method === "HEAD" ? 200 : 304, headers });
    }
    if (ranged && object.range) {
      const { offset = 0, length = object.size - offset } = object.range;
      headers.set("content-range", `bytes ${offset}-${offset + length - 1}/${object.size}`);
      headers.set("content-length", String(length));
      return new Response(object.body, { status: 206, headers });
    }
    headers.set("content-length", String(object.size));
    const response = new Response(object.body, { status: 200, headers });
    if (cache && request.method === "GET") {
      const cached = response.clone();
      const shared = new Response(cached.body, cached);
      shared.headers.set("cache-control", "public, max-age=604800, immutable");
      ctx?.waitUntil?.(cache.put(cacheKey, shared));
    }
    return response;
  },
};
