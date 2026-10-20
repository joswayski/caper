import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { afterEach, test, vi } from "vitest";
import { fetchableUrl, safeFetch } from "./safe-fetch.mjs";
import { parseHead } from "./meta.mjs";
import { twitchTarget, unfurl, xToken, youtubeVideo } from "./sites.mjs";
import worker, { signPath } from "./worker.mjs";

const fixture = (name) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");
const env = {
  EMBED_API_SECRET: "api-secret-0123456789abcdef0123456789",
  EMBED_MEDIA_SECRET: "media-secret-0123456789abcdef01234567",
  EMBED_ORIGIN: "https://embed.test",
};

// Routes fetch() to canned responses: { "https://host/path": Response | () => Response }.
function serve(routes) {
  const seen = [];
  vi.stubGlobal("fetch", async (input, init = {}) => {
    const url = String(input);
    seen.push({ url, init });
    const match = Object.entries(routes).find(([prefix]) => url.startsWith(prefix));
    if (!match) return new Response("not found", { status: 404 });
    const response = typeof match[1] === "function" ? match[1](url, init) : match[1];
    // A fresh, untee'd body each time, like a real network response.
    const body = response.body ? await response.clone().arrayBuffer() : null;
    return new Response(body, { status: response.status, headers: response.headers });
  });
  return seen;
}

const html = (body, headers = {}) =>
  new Response(body, { headers: { "content-type": "text/html; charset=utf-8", ...headers } });
const jsonResponse = (body) => new Response(body, { headers: { "content-type": "application/json" } });

afterEach(() => vi.unstubAllGlobals());

async function unfurlRequest(url, secret = env.EMBED_API_SECRET) {
  const response = await worker.fetch(
    new Request("https://embed.test/v1/unfurl", {
      method: "POST",
      headers: { authorization: `Bearer ${secret}`, "content-type": "application/json" },
      body: JSON.stringify({ url }),
    }),
    env,
  );
  return { status: response.status, body: await response.json() };
}

test("only public http(s) URLs on default ports are fetchable", () => {
  for (const ok of ["https://example.com/a", "http://news.example.co.uk/x?y=1", "https://example.com:443/"]) {
    assert.ok(fetchableUrl(ok), ok);
  }
  for (const bad of [
    "javascript:alert(1)",
    "file:///etc/passwd",
    "ftp://example.com/",
    "http://localhost/",
    "http://127.0.0.1/",
    "http://2130706433/",
    "http://0x7f.1/",
    "http://169.254.169.254/latest/meta-data/",
    "http://[::1]/",
    "http://[fd00::1]/",
    "http://10.0.0.1:8080/",
    "https://example.com:8443/",
    "https://user:pass@example.com/",
    "http://valkey.caper.svc.cluster.local/",
    "http://metadata.google.internal/",
    "http://printer/",
  ]) {
    assert.equal(fetchableUrl(bad), null, bad);
  }
});

test("redirects are followed only to fetchable URLs, at most five hops", async () => {
  serve({
    "https://a.example/": new Response(null, { status: 302, headers: { location: "https://b.example/" } }),
    "https://b.example/": new Response(null, { status: 301, headers: { location: "http://169.254.169.254/" } }),
  });
  await assert.rejects(safeFetch("https://a.example/"), /redirect not allowed/);

  serve({ "https://loop.example/": new Response(null, { status: 302, headers: { location: "/again" } }) });
  await assert.rejects(safeFetch("https://loop.example/"), /too many redirects/);
});

test("head parsing decodes entities and keeps every og:image in order", () => {
  const { meta, title } = parseHead(`<html><head>
    <title> Fish &amp; Chips </title>
    <meta property="og:title" content="It&#39;s &quot;here&quot; &#x1F41F;">
    <meta content='second' name="description">
    <meta property="og:image" content="/one.png"><meta property="og:image" content="https://cdn.example/two.png">
    </head><body><meta property="og:title" content="ignored"></body>`);
  assert.equal(title, "Fish & Chips");
  assert.equal(meta.get("og:title"), 'It\'s "here" 🐟');
  assert.equal(meta.get("description"), "second");
  assert.deepEqual(meta.get("og:image:all"), ["/one.png", "https://cdn.example/two.png"]);
});

test("generic pages become a title, description and large image card", async () => {
  serve({
    "https://www.news.example/story": html(`<head>
      <meta property="og:site_name" content="Example News">
      <meta property="og:title" content="A headline">
      <meta property="og:description" content="What happened, briefly.">
      <meta property="og:image" content="/lead.jpg"><meta property="og:image:width" content="1200">
      <meta property="og:image:height" content="630"><meta name="twitter:card" content="summary_large_image">
      </head>`),
  });
  const embed = await unfurl(new URL("https://www.news.example/story"));
  assert.deepEqual(embed, {
    version: 1,
    url: "https://www.news.example/story",
    site: { name: "Example News", domain: "news.example" },
    blocks: [
      { type: "title", text: "A headline", url: "https://www.news.example/story" },
      { type: "text", text: "What happened, briefly." },
      { type: "images", items: [{ url: "https://www.news.example/lead.jpg", width: 1200, height: 630 }] },
    ],
  });
});

test("generic pages with a direct video file get a playable video block", async () => {
  serve({
    "https://clips.example/1": html(`<head><meta property="og:title" content="Clip">
      <meta property="og:video" content="https://media.clips.example/1.mp4">
      <meta property="og:video:type" content="video/mp4"><meta property="og:image" content="https://media.clips.example/1.jpg">
      </head>`),
  });
  const embed = await unfurl(new URL("https://clips.example/1"));
  assert.deepEqual(embed.blocks.at(-1), {
    type: "video",
    url: "https://media.clips.example/1.mp4",
    poster: "https://media.clips.example/1.jpg",
  });
});

test("iframe-only players from arbitrary pages are ignored, and empty pages get no embed", async () => {
  serve({
    "https://player.example/": html(`<head><meta property="og:video" content="https://player.example/embed">
      <meta property="og:video:type" content="text/html"></head>`),
    "https://bare.example/": html("<head></head><body>hi</body>"),
    "https://file.example/": new Response("%PDF", { headers: { "content-type": "application/pdf" } }),
  });
  assert.equal(await unfurl(new URL("https://player.example/")), null);
  assert.equal(await unfurl(new URL("https://bare.example/")), null);
  assert.equal(await unfurl(new URL("https://file.example/")), null);
});

test("links straight to an image become an image card", async () => {
  serve({ "https://img.example/cat.webp": new Response("RIFF", { headers: { "content-type": "image/webp" } }) });
  const embed = await unfurl(new URL("https://img.example/cat.webp"));
  assert.deepEqual(embed.blocks, [{ type: "images", items: [{ url: "https://img.example/cat.webp" }] }]);
});

test("Instagram and Facebook are fetched as Meta's own crawler", async () => {
  const seen = serve({ "https://www.instagram.com/p/": html('<head><meta property="og:title" content="Egg"></head>') });
  await unfurl(new URL("https://www.instagram.com/p/BsOGulcndj-/"));
  assert.equal(seen[0].init.headers["user-agent"], "facebookexternalhit/1.1");
});

test("Instagram previews split the poster, caption and counts", async () => {
  serve({
    "https://www.instagram.com/p/": html(`<head><meta property="og:site_name" content="Instagram">
      <meta property="og:title" content="Just An Egg &#x1f95a; on Instagram: &quot;Hello&quot;">
      <meta property="og:description" content="60M likes, 4M comments - world_record_egg on January 4, 2019: &quot;Hello&quot;.">
      <meta property="og:image" content="https://scontent.cdninstagram.com/egg.jpg"></head>`),
  });
  const embed = await unfurl(new URL("https://www.instagram.com/p/BsOGulcndj-/"));
  assert.deepEqual(embed.blocks, [
    {
      type: "author",
      name: "Just An Egg 🥚",
      handle: "@world_record_egg",
      url: "https://www.instagram.com/world_record_egg/",
    },
    { type: "text", text: "Hello" },
    { type: "images", items: [{ url: "https://scontent.cdninstagram.com/egg.jpg" }], thumbnail: true },
    { type: "footer", text: "60M likes · 4M comments", timestamp: new Date("January 4, 2019").toISOString() },
  ]);
});

test("X posts read from the syndication endpoint with the widget token", async () => {
  assert.equal(xToken("20"), "6dq1a2xwd93");
  const seen = serve({ "https://cdn.syndication.twimg.com/tweet-result": jsonResponse(fixture("tweet-video.json")) });
  const embed = await unfurl(new URL("https://twitter.com/DivineDropbear/status/1841206275088290279?s=20"));
  assert.match(seen[0].url, /id=1841206275088290279&token=[0-9a-z]+&lang=en$/);
  assert.deepEqual(embed.site, { name: "X", domain: "twitter.com" });
  const [person, body, video, foot] = embed.blocks;
  assert.deepEqual(person, {
    type: "author",
    name: "Tranquility Found",
    handle: "@DivineDropbear",
    url: "https://x.com/DivineDropbear",
    icon: "https://pbs.twimg.com/profile_images/1871316427581583360/AY58qda4_200x200.jpg",
  });
  // The trailing t.co media link is outside display_text_range.
  assert.equal(body.text.endsWith("#cockatoo"), true);
  assert.equal(video.type, "video");
  assert.match(video.url, /^https:\/\/video\.twimg\.com\/.*1280x720.*\.mp4/);
  assert.equal(video.width, 1920);
  assert.equal(video.durationMs, 29626);
  assert.equal(foot.type, "footer");
  assert.equal(foot.timestamp, "2024-10-01T19:59:18.000Z");
});

test("X photo, GIF and quote posts", async () => {
  serve({ "https://cdn.syndication.twimg.com/": jsonResponse(fixture("tweet-photos.json")) });
  const photos = await unfurl(new URL("https://x.com/a/status/1848831595014459513"));
  const gallery = photos.blocks.find((block) => block.type === "images");
  assert.equal(gallery.items.length, 3);
  assert.match(gallery.items[0].url, /^https:\/\/pbs\.twimg\.com\/media\/.*\?name=large$/);

  serve({ "https://cdn.syndication.twimg.com/": jsonResponse(fixture("tweet-gif.json")) });
  const gif = await unfurl(new URL("https://vxtwitter.com/a/status/2031318660011188567"));
  assert.equal(gif.blocks.find((block) => block.type === "video").gif, true);

  serve({ "https://cdn.syndication.twimg.com/": jsonResponse(fixture("tweet-quote.json")) });
  const quote = await unfurl(new URL("https://x.com/i/status/1902059299591090335"));
  assert.match(quote.blocks.find((block) => block.type === "text").text, /^Quoting /);
});

test("deleted or missing X posts get no embed and no generic fallback", async () => {
  const seen = serve({ "https://cdn.syndication.twimg.com/": jsonResponse('{"__typename":"TweetTombstone"}') });
  assert.equal(await unfurl(new URL("https://x.com/a/status/123")), null);
  assert.equal(seen.length, 1);
});

test("TikTok posts use the page's own data and point video at this Worker", async () => {
  serve({ "https://www.tiktok.com/@i/video/6718335390845095173": html(fixture("tiktok-video.html")) });
  const embed = await unfurl(new URL("https://www.tiktok.com/@scout2015/video/6718335390845095173?lang=en"));
  assert.deepEqual(embed.site, { name: "TikTok", domain: "tiktok.com" });
  assert.equal(embed.blocks[0].handle, "@scout2015");
  assert.deepEqual(embed.blocks[2], {
    type: "video",
    url: "/tiktok/6718335390845095173/video",
    poster: "/tiktok/6718335390845095173/cover",
    width: 576,
    height: 1024,
    durationMs: 10000,
  });
});

test("TikTok share links resolve to the post first", async () => {
  serve({
    "https://vm.tiktok.com/ZMabc/": new Response(null, {
      status: 301,
      headers: { location: "https://www.tiktok.com/@scout2015/video/6718335390845095173?_r=1" },
    }),
    "https://www.tiktok.com/@scout2015/video/": html("<head></head>"),
    "https://www.tiktok.com/@i/video/6718335390845095173": html(fixture("tiktok-video.html")),
  });
  const embed = await unfurl(new URL("https://vm.tiktok.com/ZMabc/"));
  assert.equal(embed.blocks.find((block) => block.type === "video").url, "/tiktok/6718335390845095173/video");
});

test("YouTube links become a player with the video's thumbnail", async () => {
  assert.deepEqual(youtubeVideo(new URL("https://youtu.be/dQw4w9WgXcQ?t=1m30s")), {
    id: "dQw4w9WgXcQ",
    vertical: false,
    start: 90,
  });
  assert.equal(youtubeVideo(new URL("https://www.youtube.com/shorts/dQw4w9WgXcQ")).vertical, true);
  assert.equal(youtubeVideo(new URL("https://www.youtube.com/watch?v=bad")), null);
  serve({ "https://www.youtube.com/oembed": jsonResponse(fixture("youtube-oembed.json")) });
  const embed = await unfurl(new URL("https://m.youtube.com/watch?v=dQw4w9WgXcQ&feature=share"));
  assert.deepEqual(embed.blocks, [
    { type: "author", name: "Rick Astley", url: "https://www.youtube.com/@RickAstleyYT" },
    {
      type: "title",
      text: "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)",
      url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    },
    {
      type: "player",
      provider: "youtube",
      id: "dQw4w9WgXcQ",
      poster: "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
      width: 1280,
      height: 720,
    },
  ]);
});

test("Twitch links name the clip, VOD or channel to play", () => {
  assert.equal(twitchTarget(new URL("https://clips.twitch.tv/FunnyClipSlug-abc")), "clip:FunnyClipSlug-abc");
  assert.equal(twitchTarget(new URL("https://www.twitch.tv/someone/clip/Slug_1")), "clip:Slug_1");
  assert.equal(twitchTarget(new URL("https://www.twitch.tv/videos/2200000000")), "video:2200000000");
  assert.equal(twitchTarget(new URL("https://m.twitch.tv/Streamer")), "channel:streamer");
  assert.equal(twitchTarget(new URL("https://www.twitch.tv/directory/all")), null);
});

test("the unfurl route requires the API secret and signs every image", async () => {
  assert.equal((await unfurlRequest("https://x.com/a/status/1", "wrong")).status, 401);
  serve({ "https://cdn.syndication.twimg.com/": jsonResponse(fixture("tweet-photos.json")) });
  const { status, body } = await unfurlRequest("https://x.com/a/status/1848831595014459513");
  assert.equal(status, 200);
  const icon = body.embed.blocks[0].icon;
  assert.match(icon, /^https:\/\/embed\.test\/m\/[A-Za-z0-9_-]{43}\/i\?u=https%3A%2F%2Fpbs\.twimg\.com%2F/);
  const path = icon.slice("https://embed.test/m/".length + 43);
  assert.equal(icon.split("/")[4], await signPath(env.EMBED_MEDIA_SECRET, path));
  assert.equal((await unfurlRequest("http://169.254.169.254/")).body.embed, null);
});

test("the image proxy serves only signed, real images and never SVG", async () => {
  const target = "https://pbs.example/a.png";
  const path = `/i?u=${encodeURIComponent(target)}`;
  const good = `https://embed.test/m/${await signPath(env.EMBED_MEDIA_SECRET, path)}${path}`;
  serve({ [target]: new Response("PNG", { headers: { "content-type": "image/png", "content-length": "3" } }) });
  const response = await worker.fetch(new Request(good), env);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "image/png");
  assert.equal(response.headers.get("content-security-policy"), "default-src 'none'; sandbox");
  assert.equal(await response.text(), "PNG");

  const forged = `https://embed.test/m/${"A".repeat(43)}${path}`;
  assert.equal((await worker.fetch(new Request(forged), env)).status, 404);

  serve({ [target]: new Response("<svg onload=alert(1)>", { headers: { "content-type": "image/svg+xml" } }) });
  assert.equal((await worker.fetch(new Request(good), env)).status, 415);
});

test("TikTok video redirects to the fresh CDN file; other hosts are refused", async () => {
  const path = "/tiktok/6718335390845095173/video";
  const url = `https://embed.test/m/${await signPath(env.EMBED_MEDIA_SECRET, path)}${path}`;
  serve({
    "https://www.tiktok.com/@i/video/": html(fixture("tiktok-video.html")),
    "https://www.tiktok.com/aweme/v1/play/": new Response(null, {
      status: 302,
      headers: { location: "https://v16m-default.tiktokcdn-us.com/abc/video.mp4" },
    }),
  });
  const response = await worker.fetch(new Request(url), env);
  assert.equal(response.status, 302);
  assert.equal(response.headers.get("location"), "https://v16m-default.tiktokcdn-us.com/abc/video.mp4");

  serve({
    "https://www.tiktok.com/@i/video/": html(fixture("tiktok-video.html")),
    "https://www.tiktok.com/aweme/v1/play/": new Response(null, {
      status: 302,
      headers: { location: "http://169.254.169.254/" },
    }),
  });
  assert.equal((await worker.fetch(new Request(url), env)).status, 502);
});
