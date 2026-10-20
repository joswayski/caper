// Link previews for Caper at embed.caper.chat. Runs on Cloudflare, outside the
// AWS network, so fetching links people post can never reach Caper's cluster.
//
//   POST /v1/unfurl {url}        API only (Bearer EMBED_API_SECRET): the
//                                 embed for one link, or {embed: null}
//   GET  /m/{sig}/i?u={url}       a preview image, fetched and cached here so
//                                 the site never sees who is reading
//   GET  /m/{sig}/tiktok/{id}/…   TikTok and Instagram video, covers and photos,
//   GET  /m/{sig}/instagram/{code}/…  re-resolved on each view because their
//                                 addresses expire
//   GET  /play/{provider}/{id}    a page holding only that provider's player,
//                                 for native apps' web views (see player.mjs)
//
// Media links are signed (HMAC-SHA256 with EMBED_MEDIA_SECRET over the path
// after the signature) so this Worker only ever fetches media it handed out.

import { contentType, discard, fetchableUrl, safeFetch } from "./safe-fetch.mjs";
import { playerPage, playPath } from "./player.mjs";
import { instagramFiles, tiktokItem, tiktokPlayUrl, unfurl } from "./sites.mjs";

const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"]);
const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
const UNFURL_TTL = 30 * 60;
const MISS_TTL = 5 * 60;
const TIKTOK_TTL = 10 * 60;
const encoder = new TextEncoder();

function base64url(bytes) {
  let text = "";
  for (const byte of new Uint8Array(bytes)) text += String.fromCharCode(byte);
  return btoa(text).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

async function hmac(secret) {
  return crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
    "verify",
  ]);
}

export async function signPath(secret, path) {
  return base64url(await crypto.subtle.sign("HMAC", await hmac(secret), encoder.encode(path)));
}

async function verifyPath(secret, path, signature) {
  if (!/^[A-Za-z0-9_-]{43}$/.test(signature)) return false;
  const bytes = Uint8Array.from(atob(signature.replaceAll("-", "+").replaceAll("_", "/") + "="), (c) =>
    c.charCodeAt(0),
  );
  return crypto.subtle.verify("HMAC", await hmac(secret), bytes, encoder.encode(path));
}

async function sameSecret(given, expected) {
  if (!expected || typeof given !== "string") return false;
  // Compare fixed-length digests in constant time.
  const [a, b] = await Promise.all(
    [given, expected].map(
      async (value) => new Uint8Array(await crypto.subtle.digest("SHA-256", encoder.encode(value))),
    ),
  );
  let difference = 0;
  for (let index = 0; index < a.length; index++) difference |= a[index] ^ b[index];
  return difference === 0;
}

function respond(status, body, headers = {}) {
  return new Response(body === undefined ? null : JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
      ...headers,
    },
  });
}

// Rewrites every media field to a signed URL on this Worker, so clients only
// ever load preview images from embed.caper.chat. Direct video files stay on
// the site (X, generic pages): the player streams them from there.
async function signMedia(embed, origin, secret) {
  const media = async (value) => {
    if (!value) return value;
    const path = value.startsWith("/") ? value : `/i?u=${encodeURIComponent(value)}`;
    return `${origin}/m/${await signPath(secret, path)}${path}`;
  };
  for (const block of embed.blocks) {
    if (block.type === "author" && block.icon) block.icon = await media(block.icon);
    if (block.type === "images") for (const item of block.items) item.url = await media(item.url);
    if (block.type === "video" || block.type === "player") {
      if (block.poster) block.poster = await media(block.poster);
      if (block.type === "video" && block.url.startsWith("/")) block.url = await media(block.url);
    }
    if (block.type === "player") block.play = `${origin}${playPath(block)}`;
  }
  return embed;
}

async function cacheKey(origin, kind, value) {
  const digest = base64url(await crypto.subtle.digest("SHA-256", encoder.encode(value)));
  return new Request(`${origin}/cache/${kind}/${digest}`);
}

async function cached(origin, kind, value, ttl, ctx, produce) {
  const cache = globalThis.caches?.default;
  const key = cache && (await cacheKey(origin, kind, value));
  if (cache) {
    const hit = await cache.match(key);
    if (hit) return hit.json();
  }
  const result = await produce();
  if (cache) {
    const entry = new Response(JSON.stringify(result), {
      headers: { "content-type": "application/json", "cache-control": `max-age=${result ? ttl : MISS_TTL}` },
    });
    ctx?.waitUntil?.(cache.put(key, entry));
  }
  return result;
}

async function handleUnfurl(request, env, ctx, origin) {
  const token = request.headers.get("authorization")?.replace(/^Bearer /, "");
  if (!(await sameSecret(token, env.EMBED_API_SECRET))) return respond(401, { error: "unauthorized" });
  let input;
  try {
    input = await request.json();
  } catch {
    return respond(400, { error: "invalid json" });
  }
  const url = typeof input?.url === "string" && input.url.length <= 2048 ? fetchableUrl(input.url) : null;
  if (!url) return respond(200, { embed: null });
  const embed = await cached(origin, "unfurl-v1", url.href, UNFURL_TTL, ctx, () => unfurl(url));
  return respond(200, {
    embed: embed ? await signMedia(structuredClone(embed), origin, env.EMBED_MEDIA_SECRET) : null,
  });
}

function mediaHeaders(type, length) {
  const headers = new Headers({
    "content-type": type,
    "cache-control": "public, max-age=86400",
    "x-content-type-options": "nosniff",
    "content-security-policy": "default-src 'none'; sandbox",
    "cross-origin-resource-policy": "cross-origin",
    "access-control-allow-origin": "*",
  });
  if (length) headers.set("content-length", length);
  return headers;
}

// Streams an upstream image back, refusing anything that is not an image or
// is larger than MAX_IMAGE_BYTES. SVG is never served: it can carry script.
async function proxyImage(target, request, ctx) {
  const cache = globalThis.caches?.default;
  const key = new Request(new URL(request.url).href);
  if (cache) {
    const hit = await cache.match(key);
    if (hit) return hit;
  }
  let upstream;
  try {
    ({ response: upstream } = await safeFetch(target, { headers: { accept: "image/avif,image/webp,image/*;q=0.8" } }));
  } catch {
    return new Response(null, { status: 502, headers: { "cache-control": "no-store" } });
  }
  const type = contentType(upstream);
  const length = Number(upstream.headers.get("content-length"));
  if (!upstream.ok || !IMAGE_TYPES.has(type) || length > MAX_IMAGE_BYTES) {
    discard(upstream);
    return new Response(null, { status: upstream.ok ? 415 : 502, headers: { "cache-control": "no-store" } });
  }
  let seen = 0;
  const limited = upstream.body.pipeThrough(
    new TransformStream({
      transform(chunk, controller) {
        seen += chunk.byteLength;
        if (seen > MAX_IMAGE_BYTES) controller.error(new Error("image too large"));
        else controller.enqueue(chunk);
      },
    }),
  );
  const response = new Response(limited, { headers: mediaHeaders(type, length || undefined) });
  if (cache && length && length <= MAX_IMAGE_BYTES) {
    const copy = response.clone();
    copy.headers.set("cache-control", "public, max-age=604800");
    ctx?.waitUntil?.(cache.put(key, copy));
  }
  return response;
}

async function tiktok(id, origin, ctx) {
  return cached(origin, "tiktok-v1", id, TIKTOK_TTL, ctx, async () => {
    const item = await tiktokItem(id);
    if (!item) return null;
    return {
      play: tiktokPlayUrl(item),
      cover: item.video?.cover || item.video?.originCover || null,
      images: (item.imagePost?.images ?? []).map((image) => image.imageURL?.urlList?.[0] ?? null),
    };
  });
}

const INSTAGRAM_CDN = /(^|\.)(cdninstagram\.com|fbcdn\.net)$/;

// Instagram media: images and posters through the image proxy; video as a
// redirect to the fresh CDN file, which players fetch directly.
async function handleInstagram(code, part, request, ctx, origin) {
  const files = await cached(origin, "instagram-v1", code, TIKTOK_TTL, ctx, () => instagramFiles(code)).catch(
    () => null,
  );
  if (!files) return new Response(null, { status: 404, headers: { "cache-control": "no-store" } });
  if (part === "avatar")
    return files.avatar ? proxyImage(files.avatar, request, ctx) : new Response(null, { status: 404 });
  const [kind, index] = part.split("/");
  const entry = files.media[Number(index)];
  const target = kind === "image" ? entry?.image : entry?.video;
  if (!target) return new Response(null, { status: 404 });
  if (kind === "image") return proxyImage(target, request, ctx);
  const location = fetchableUrl(target);
  if (!location || location.protocol !== "https:" || !INSTAGRAM_CDN.test(location.hostname)) {
    return new Response(null, { status: 502, headers: { "cache-control": "no-store" } });
  }
  return new Response(null, {
    status: 302,
    headers: { location: location.href, "cache-control": "private, max-age=300", "access-control-allow-origin": "*" },
  });
}

const TIKTOK_CDN = /(^|\.)(tiktokcdn(-us|-eu)?\.com|tiktokv\.com|tiktok\.com|byteoversea\.com|ibyteimg\.com)$/;

async function handleTikTok(id, part, request, ctx, origin) {
  const item = await tiktok(id, origin, ctx).catch(() => null);
  if (!item) return new Response(null, { status: 404, headers: { "cache-control": "no-store" } });
  if (part === "cover") return item.cover ? proxyImage(item.cover, request, ctx) : new Response(null, { status: 404 });
  const image = /^image\/(\d{1,2})$/.exec(part);
  if (image) {
    const target = item.images[Number(image[1])];
    return target ? proxyImage(target, request, ctx) : new Response(null, { status: 404 });
  }
  if (part !== "video" || !item.play) return new Response(null, { status: 404 });
  // The play address answers with a redirect to a short-lived CDN file that
  // players fetch directly (with range requests for seeking).
  const upstream = await fetch(item.play, {
    headers: { "user-agent": "Mozilla/5.0", accept: "*/*" },
    redirect: "manual",
  }).catch(() => null);
  const location = upstream?.headers.get("location");
  discard(upstream);
  const target = location && fetchableUrl(location, item.play);
  if (!target || target.protocol !== "https:" || !TIKTOK_CDN.test(target.hostname)) {
    return new Response(null, { status: 502, headers: { "cache-control": "no-store" } });
  }
  return new Response(null, {
    status: 302,
    headers: { location: target.href, "cache-control": "private, max-age=300", "access-control-allow-origin": "*" },
  });
}

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const origin = env.EMBED_ORIGIN ?? url.origin;
    if (url.pathname === "/v1/unfurl") {
      return request.method === "POST" ? handleUnfurl(request, env, ctx, origin) : respond(405, { error: "method" });
    }
    if (url.pathname.startsWith("/play/")) {
      return request.method === "GET" || request.method === "HEAD"
        ? playerPage(url)
        : respond(405, { error: "method" });
    }
    const media = /^\/m\/([A-Za-z0-9_-]{43})(\/.*)$/.exec(url.pathname);
    if (!media || (request.method !== "GET" && request.method !== "HEAD")) return respond(404, { error: "not found" });
    const path = `${media[2]}${url.search}`;
    if (!(await verifyPath(env.EMBED_MEDIA_SECRET, path, media[1]))) return respond(404, { error: "not found" });
    if (media[2] === "/i") {
      const target = url.searchParams.get("u");
      return target ? proxyImage(target, request, ctx) : respond(404, { error: "not found" });
    }
    const tiktokPath = /^\/tiktok\/(\d{8,25})\/(video|cover|image\/\d{1,2})$/.exec(media[2]);
    if (tiktokPath) return handleTikTok(tiktokPath[1], tiktokPath[2], request, ctx, origin);
    const instagramPath = /^\/instagram\/([A-Za-z0-9_-]{5,40})\/(avatar|image\/\d{1,2}|video\/\d{1,2})$/.exec(media[2]);
    if (instagramPath) return handleInstagram(instagramPath[1], instagramPath[2], request, ctx, origin);
    return respond(404, { error: "not found" });
  },
};
