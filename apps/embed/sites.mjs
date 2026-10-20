// Readers that turn a posted link into embed blocks. Sites with good preview
// tags go through `generic`; X, TikTok and the video players get their own
// reader because their pages give crawlers little or nothing. Each reader
// returns an embed or null; a null from a site reader falls back to `generic`.
//
// Media fields hold the upstream URL (or a path on this Worker for media that
// has to be re-resolved on every view); worker.mjs signs and proxies them.

import {
  author,
  clean,
  compactNumber,
  dimension,
  embed,
  footer,
  httpsUrl,
  images,
  LIMITS,
  milliseconds,
  text,
  title,
} from "./blocks.mjs";
import { decodeEntities, parseHead } from "./meta.mjs";
import { contentType, discard, readText, safeFetch, USER_AGENT } from "./safe-fetch.mjs";

const BROWSER_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";
const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"]);
const VIDEO_TYPES = new Set(["video/mp4", "video/webm"]);
const HTML_TYPES = new Set(["text/html", "application/xhtml+xml"]);
const HTML_LIMIT = 1024 * 1024;
const PAGE_DATA_LIMIT = 3 * 1024 * 1024;

export function hostOf(url) {
  return url.hostname.toLowerCase().replace(/^www\./, "");
}

function onDomain(host, domains) {
  return domains.some((domain) => host === domain || host.endsWith(`.${domain}`));
}

function site(url, name) {
  return { name, domain: hostOf(url) };
}

async function json(url, headers) {
  const { response } = await safeFetch(url, { headers: { accept: "application/json", ...headers } });
  if (!response.ok) {
    discard(response);
    return null;
  }
  try {
    return JSON.parse(await readText(response, PAGE_DATA_LIMIT));
  } catch {
    return null;
  }
}

// X / Twitter. The same public endpoint X's own embed widget reads, which
// needs no account; links from fix services (vxtwitter and friends) count too.

const X_DOMAINS = ["x.com", "twitter.com", "vxtwitter.com", "fxtwitter.com", "fixupx.com", "fixvx.com"];
const X_STATUS = /^\/(?:[A-Za-z0-9_]{1,15}|i(?:\/web)?)\/status(?:es)?\/(\d{1,20})(?:\/|$)/;

export function xToken(id) {
  return ((Number(id) / 1e15) * Math.PI).toString(36).replace(/(0+|\.)/g, "");
}

function tweetText(tweet) {
  const points = [...(tweet.text ?? "")];
  const [start, end] = Array.isArray(tweet.display_text_range) ? tweet.display_text_range : [0, points.length];
  let value = points.slice(start, end).join("");
  for (const link of tweet.entities?.urls ?? []) {
    if (link.url && link.expanded_url) value = value.replaceAll(link.url, link.expanded_url);
  }
  return decodeEntities(value);
}

function bestMp4(variants = []) {
  const mp4 = variants.filter((variant) => variant.content_type === "video/mp4" && httpsUrl(variant.url));
  // Highest quality that still streams comfortably on mobile data.
  const fitting = mp4.filter((variant) => (variant.bitrate ?? 0) <= 2_500_000);
  const pool = fitting.length ? fitting : mp4;
  return pool.sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0))[0]?.url ?? null;
}

export function tweetBlocks(tweet) {
  const user = tweet.user ?? {};
  const handle = user.screen_name;
  const blocks = [
    author(user.name ?? handle, {
      handle: handle ? `@${handle}` : undefined,
      url: handle ? `https://x.com/${encodeURIComponent(handle)}` : undefined,
      icon: httpsUrl(user.profile_image_url_https?.replace("_normal.", "_200x200.")) ?? undefined,
    }),
    text(tweetText(tweet)),
  ];
  const media = tweet.mediaDetails ?? [];
  const photos = media.filter((item) => item.type === "photo" && httpsUrl(item.media_url_https));
  const video = media.find((item) => item.type === "video" || item.type === "animated_gif");
  const videoUrl = video && bestMp4(video.video_info?.variants);
  if (videoUrl) {
    blocks.push({
      type: "video",
      url: videoUrl,
      poster: httpsUrl(video.media_url_https) ?? undefined,
      width: dimension(video.original_info?.width),
      height: dimension(video.original_info?.height),
      durationMs: milliseconds(video.video_info?.duration_millis),
      gif: video.type === "animated_gif" || undefined,
    });
  } else if (photos.length) {
    blocks.push(
      images(
        photos.map((photo) => ({
          url: `${photo.media_url_https}?name=large`,
          width: dimension(photo.original_info?.width),
          height: dimension(photo.original_info?.height),
          alt: photo.ext_alt_text,
        })),
      ),
    );
  }
  const quoted = tweet.quoted_tweet;
  if (quoted?.user) {
    blocks.push(
      text(`Quoting ${quoted.user.name} (@${quoted.user.screen_name})\n${tweetText(quoted)}`, LIMITS.description),
    );
  }
  const counts = [
    compactNumber(tweet.favorite_count) && `${compactNumber(tweet.favorite_count)} likes`,
    compactNumber(tweet.conversation_count) && `${compactNumber(tweet.conversation_count)} replies`,
  ].filter(Boolean);
  blocks.push(footer(counts.join(" · "), tweet.created_at));
  return blocks;
}

async function readX(url, match) {
  const id = match[1];
  const data = await json(`https://cdn.syndication.twimg.com/tweet-result?id=${id}&token=${xToken(id)}&lang=en`, {
    "user-agent": BROWSER_UA,
  });
  const tweet = data?.__typename === "TweetWithVisibilityResults" ? data.tweet : data;
  if (tweet?.__typename !== "Tweet") return null;
  return embed(url.href, site(url, "X"), tweetBlocks(tweet));
}

// TikTok. The post page embeds its data as JSON for the web app; that JSON
// carries the video's play address. Video and image addresses expire, so the
// embed points at /tiktok/{id}/... on this Worker, which resolves them fresh.

const TIKTOK_POST = /^\/(?:@[^/]+\/(?:video|photo)|embed(?:\/v2)?|video|v)\/(\d{8,25})(?:\.html)?(?:\/|$)/;

export function tiktokItemFromPage(html) {
  const start = html.indexOf('<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">');
  if (start < 0) return null;
  const from = html.indexOf(">", start) + 1;
  const end = html.indexOf("</script>", from);
  if (end < 0) return null;
  try {
    const detail = JSON.parse(html.slice(from, end))?.__DEFAULT_SCOPE__?.["webapp.video-detail"];
    if (detail?.statusCode !== 0) return null;
    return detail.itemInfo?.itemStruct ?? null;
  } catch {
    return null;
  }
}

export async function tiktokItem(id) {
  const { response } = await safeFetch(`https://www.tiktok.com/@i/video/${id}`, {
    headers: { "user-agent": BROWSER_UA, accept: "text/html", "accept-language": "en-US,en;q=0.9" },
  });
  if (!response.ok) {
    discard(response);
    return null;
  }
  return tiktokItemFromPage(await readText(response, PAGE_DATA_LIMIT));
}

export function tiktokPlayUrl(item) {
  const h264 = (item.video?.bitrateInfo ?? []).filter((rate) => !String(rate.CodecType ?? "").includes("265"));
  for (const rate of h264) {
    const play = rate.PlayAddr?.UrlList?.find((candidate) => candidate.includes("/aweme/v1/play/"));
    if (play) return play;
  }
  return httpsUrl(item.video?.playAddr) ?? null;
}

export function tiktokBlocks(item, id) {
  const person = item.author ?? {};
  const blocks = [
    author(person.nickname || person.uniqueId, {
      handle: person.uniqueId ? `@${person.uniqueId}` : undefined,
      url: person.uniqueId ? `https://www.tiktok.com/@${encodeURIComponent(person.uniqueId)}` : undefined,
      icon: httpsUrl(person.avatarMedium ?? person.avatarThumb) ?? undefined,
    }),
    text(item.desc, LIMITS.description),
  ];
  const photos = item.imagePost?.images ?? [];
  if (photos.length) {
    blocks.push(
      images(
        photos.map((photo, index) => ({
          url: `/tiktok/${id}/image/${index}`,
          width: dimension(photo.imageWidth),
          height: dimension(photo.imageHeight),
        })),
      ),
    );
  } else if (tiktokPlayUrl(item)) {
    blocks.push({
      type: "video",
      url: `/tiktok/${id}/video`,
      poster: `/tiktok/${id}/cover`,
      width: dimension(item.video?.width),
      height: dimension(item.video?.height),
      durationMs: milliseconds(Number(item.video?.duration) * 1000),
    });
  }
  const stats = item.stats ?? {};
  const counts = [
    compactNumber(stats.diggCount) && `${compactNumber(stats.diggCount)} likes`,
    compactNumber(stats.commentCount) && `${compactNumber(stats.commentCount)} comments`,
  ].filter(Boolean);
  const created = Number(item.createTime);
  blocks.push(footer(counts.join(" · "), created > 0 ? new Date(created * 1000).toISOString() : undefined));
  return blocks;
}

async function tiktokId(url) {
  const direct = TIKTOK_POST.exec(url.pathname);
  if (direct) return direct[1];
  // Share links (vm.tiktok.com/…, tiktok.com/t/…) redirect to the post.
  if (!/^(vm|vt)\./.test(url.hostname) && !url.pathname.startsWith("/t/")) return null;
  const { response, url: landed } = await safeFetch(url.href, { headers: { "user-agent": BROWSER_UA } });
  discard(response);
  return TIKTOK_POST.exec(landed.pathname)?.[1] ?? null;
}

async function readTikTok(url) {
  const id = await tiktokId(url);
  if (!id) return null;
  const item = await tiktokItem(id);
  return item ? embed(url.href, site(url, "TikTok"), tiktokBlocks(item, id)) : null;
}

// Players. These sites only offer their own player; the card shows their
// thumbnail and clients load the player from embed.caper.chat on Play.

const YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/;

export function youtubeVideo(url) {
  const host = hostOf(url);
  let id = null;
  let vertical = false;
  if (host === "youtu.be") id = url.pathname.split("/")[1];
  else if (url.pathname === "/watch") id = url.searchParams.get("v");
  else {
    const [, kind, value] = url.pathname.split("/");
    if (["shorts", "live", "embed", "v"].includes(kind)) id = value;
    vertical = kind === "shorts";
  }
  if (!id || !YOUTUBE_ID.test(id)) return null;
  const time = url.searchParams.get("t") ?? url.searchParams.get("start");
  const parts = time && /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s?)?$/.exec(time);
  const start = parts ? Number(parts[1] ?? 0) * 3600 + Number(parts[2] ?? 0) * 60 + Number(parts[3] ?? 0) : 0;
  return { id, vertical, start: start > 0 ? start : undefined };
}

async function readYouTube(url) {
  const video = youtubeVideo(url);
  if (!video) return null;
  const watch = `https://www.youtube.com/watch?v=${video.id}`;
  const data = await json(`https://www.youtube.com/oembed?format=json&url=${encodeURIComponent(watch)}`);
  if (!data?.title) return null;
  return embed(url.href, site(url, "YouTube"), [
    author(data.author_name, { url: httpsUrl(data.author_url) ?? undefined }),
    title(data.title, watch),
    {
      type: "player",
      provider: "youtube",
      id: video.id,
      poster: `https://i.ytimg.com/vi/${video.id}/hqdefault.jpg`,
      width: video.vertical ? 720 : 1280,
      height: video.vertical ? 1280 : 720,
      start: video.start,
    },
  ]);
}

async function readVimeo(url) {
  const id = /^\/(?:video\/)?(\d{1,12})(?:\/|$)/.exec(url.pathname)?.[1];
  if (!id) return null;
  const data = await json(`https://vimeo.com/api/oembed.json?url=${encodeURIComponent(`https://vimeo.com/${id}`)}`);
  if (!data?.title) return null;
  return embed(url.href, site(url, "Vimeo"), [
    author(data.author_name, { url: httpsUrl(data.author_url) ?? undefined }),
    title(data.title, `https://vimeo.com/${id}`),
    {
      type: "player",
      provider: "vimeo",
      id: String(data.video_id ?? id),
      poster: httpsUrl(data.thumbnail_url) ?? undefined,
      width: dimension(data.width),
      height: dimension(data.height),
    },
  ]);
}

export function twitchTarget(url) {
  const host = hostOf(url);
  const parts = url.pathname.split("/").filter(Boolean);
  if (host === "clips.twitch.tv" && /^[A-Za-z0-9_-]{3,100}$/.test(parts[0] ?? "")) return `clip:${parts[0]}`;
  if (parts[0] === "videos" && /^\d{1,15}$/.test(parts[1] ?? "")) return `video:${parts[1]}`;
  if (parts[1] === "clip" && /^[A-Za-z0-9_-]{3,100}$/.test(parts[2] ?? "")) return `clip:${parts[2]}`;
  if (parts.length === 1 && /^[A-Za-z0-9_]{3,25}$/.test(parts[0])) return `channel:${parts[0].toLowerCase()}`;
  return null;
}

async function readTwitch(url) {
  const id = twitchTarget(url);
  if (!id) return null;
  const page = await generic(url);
  const blocks = (page?.blocks ?? []).filter((block) => block.type === "title" || block.type === "text");
  const poster = page?.blocks.find((block) => block.type === "images")?.items[0]?.url;
  return embed(url.href, site(url, "Twitch"), [
    ...blocks,
    { type: "player", provider: "twitch", id, poster, width: 1280, height: 720 },
  ]);
}

// Everything else: the page's own OpenGraph / Twitter Card / <title> tags.

// Meta's sites only serve preview tags to the crawler they know.
const META_DOMAINS = ["instagram.com", "facebook.com", "fb.watch", "threads.net", "threads.com"];

export function genericBlocks(page, html) {
  const { meta, title: documentTitle } = parseHead(html);
  const first = (...keys) => {
    for (const key of keys) {
      const value = meta.get(key)?.trim();
      if (value) return value;
    }
    return null;
  };
  const heading = first("og:title", "twitter:title") ?? documentTitle;
  const description = first("og:description", "twitter:description", "description");
  const imageUrls = [...meta.get("og:image:all"), first("twitter:image", "twitter:image:src")]
    .map((value) => value && httpsUrl(value, page))
    .filter(Boolean);
  const card = first("twitter:card");
  const width = dimension(first("og:image:width"));
  const height = dimension(first("og:image:height"));
  const videoUrl = httpsUrl(first("og:video:secure_url", "og:video:url", "og:video", "twitter:player:stream"), page);
  const videoType = (first("og:video:type", "twitter:player:stream:content_type") ?? "").split(";")[0].trim();
  const playable =
    videoUrl?.startsWith("https:") &&
    (VIDEO_TYPES.has(videoType) || (!videoType && /\.(mp4|webm)(\?|$)/i.test(new URL(videoUrl).pathname)));
  if (!heading && !description && !imageUrls.length && !playable) return null;
  const blocks = [title(heading, page.href), text(description, LIMITS.description)];
  if (playable) {
    blocks.push({
      type: "video",
      url: videoUrl,
      poster: imageUrls[0],
      width: dimension(first("og:video:width")),
      height: dimension(first("og:video:height")),
    });
  } else if (imageUrls.length) {
    const large = card === "summary_large_image" || (!card && (width ?? 0) >= 600);
    blocks.push(images([{ url: imageUrls[0], width, height, alt: first("og:image:alt") }], !large));
  }
  return { siteName: clean(first("og:site_name", "application-name"), LIMITS.name), blocks };
}

export async function generic(url) {
  const meta = onDomain(hostOf(url), META_DOMAINS);
  const { response, url: page } = await safeFetch(url.href, {
    headers: {
      "user-agent": meta ? "facebookexternalhit/1.1" : USER_AGENT,
      accept: "text/html,application/xhtml+xml;q=0.9,image/*;q=0.8,video/*;q=0.5,*/*;q=0.1",
      "accept-language": "en-US,en;q=0.8",
    },
  });
  if (!response.ok) {
    discard(response);
    return null;
  }
  const type = contentType(response);
  const where = site(url, hostOf(url));
  if (IMAGE_TYPES.has(type) || (VIDEO_TYPES.has(type) && page.protocol === "https:")) {
    discard(response);
    return embed(url.href, where, [
      IMAGE_TYPES.has(type) ? images([{ url: page.href }]) : { type: "video", url: page.href },
    ]);
  }
  if (!HTML_TYPES.has(type)) {
    discard(response);
    return null;
  }
  const result = genericBlocks(page, await readText(response, HTML_LIMIT, "</head>"));
  if (!result) return null;
  return embed(url.href, { name: result.siteName ?? where.domain, domain: where.domain }, result.blocks);
}

// Instagram's tags repeat the caption in the title and description; turn them
// into the poster, the caption and the counts.
const INSTAGRAM_DESCRIPTION =
  /^(?:([\d.,]+[KMB]?) likes?, )?(?:([\d.,]+[KMB]?) comments? - )?([A-Za-z0-9._]{1,30}) on ([^:]+): "([\s\S]*)"\.?$/;

export function tidyInstagram(found) {
  const heading = found.blocks.find((block) => block.type === "title");
  const description = found.blocks.find((block) => block.type === "text");
  const parts = description && INSTAGRAM_DESCRIPTION.exec(description.text);
  if (!parts) return found;
  const [, likes, comments, handle, date, caption] = parts;
  const name = heading?.text.split(" on Instagram")[0];
  const counts = [likes && `${likes} likes`, comments && `${comments} comments`].filter(Boolean).join(" · ");
  return embed(found.url, found.site, [
    author(name || handle, { handle: `@${handle}`, url: `https://www.instagram.com/${handle}/` }),
    text(caption, LIMITS.description),
    ...found.blocks.filter((block) => block.type !== "title" && block.type !== "text"),
    footer(counts, Number.isNaN(Date.parse(date)) ? undefined : date),
  ]);
}

// Instagram's own web app asks this logged-out query for a post (the approach
// OGInstagram takes); it carries the media files, caption and counts. Media
// addresses are signed and expire, so the embed points at /instagram/{code}/…
// on this Worker. Without it, the page's tags still give an image card.

const INSTAGRAM_POST = /^\/(?:[A-Za-z0-9._]{1,30}\/)?(?:p|reels?|tv)\/([A-Za-z0-9_-]{5,40})(?:\/|$)/;
const INSTAGRAM_DOC_ID = "27128499623469141";
const INSTAGRAM_CDN = /(^|\.)(cdninstagram\.com|fbcdn\.net)$/;

export function instagramShortcode(url) {
  return INSTAGRAM_POST.exec(url.pathname)?.[1] ?? null;
}

export async function instagramItem(code) {
  const lsd = [...crypto.getRandomValues(new Uint8Array(12))]
    .map((byte) => byte.toString(36).padStart(2, "0"))
    .join("");
  const response = await fetch("https://i.instagram.com/api/graphql/", {
    method: "POST",
    headers: {
      "user-agent": "Mozilla/5.0",
      accept: "*/*",
      "content-type": "application/x-www-form-urlencoded",
      "x-fb-friendly-name": "PolarisPostRootQuery",
      "x-fb-lsd": lsd,
    },
    body: new URLSearchParams({
      variables: JSON.stringify({
        shortcode: code,
        __relay_internal__pv__PolarisAIGMMediaWebLabelEnabledrelayprovider: false,
      }),
      doc_id: INSTAGRAM_DOC_ID,
      lsd,
    }),
    redirect: "manual",
    signal: AbortSignal.timeout(6000),
  });
  // Instagram labels this JSON text/javascript.
  if (!response.ok || !["application/json", "text/javascript"].includes(contentType(response))) {
    discard(response);
    return null;
  }
  try {
    const body = JSON.parse(await readText(response, PAGE_DATA_LIMIT));
    return body?.data?.xdt_api__v1__media__shortcode__web_info?.items?.[0] ?? null;
  } catch {
    return null;
  }
}

function largest(candidates) {
  return (Array.isArray(candidates) ? candidates : [])
    .filter((candidate) => httpsUrl(candidate?.url))
    .sort((a, b) => (b.width ?? 0) * (b.height ?? 0) - (a.width ?? 0) * (a.height ?? 0))[0];
}

/** Each picture or video in a post, carousel children in order. */
export function instagramMedia(item) {
  const children = item.carousel_media?.length ? item.carousel_media : [item];
  return children.map((child) => {
    const image = largest(child.image_versions2?.candidates);
    const video = child.media_type === 2 ? largest(child.video_versions) : undefined;
    return {
      image: image?.url ?? null,
      video: video?.url ?? null,
      width: dimension(child.original_width ?? image?.width),
      height: dimension(child.original_height ?? image?.height),
    };
  });
}

export function instagramBlocks(item, code) {
  const person = item.user ?? {};
  const media = instagramMedia(item);
  const blocks = [
    author(person.full_name || person.username, {
      handle: person.username ? `@${person.username}` : undefined,
      url: person.username ? `https://www.instagram.com/${encodeURIComponent(person.username)}/` : undefined,
      icon: httpsUrl(person.profile_pic_url) ? `/instagram/${code}/avatar` : undefined,
    }),
    text(item.caption?.text, LIMITS.description),
  ];
  const [first] = media;
  if (first?.video) {
    blocks.push({
      type: "video",
      url: `/instagram/${code}/video/0`,
      poster: first.image ? `/instagram/${code}/image/0` : undefined,
      width: first.width,
      height: first.height,
    });
  } else {
    blocks.push(
      images(
        media
          .map(
            (entry, index) =>
              entry.image && { url: `/instagram/${code}/image/${index}`, width: entry.width, height: entry.height },
          )
          .filter(Boolean),
      ),
    );
  }
  // Hidden counts (and stories) come back as zero.
  const counts = [
    item.like_count > 0 && `${compactNumber(item.like_count)} likes`,
    item.comment_count > 0 && `${compactNumber(item.comment_count)} comments`,
  ].filter(Boolean);
  const taken = Number(item.taken_at);
  blocks.push(footer(counts.join(" · "), taken > 0 ? new Date(taken * 1000).toISOString() : undefined));
  return blocks;
}

/** What /instagram/{code}/… on the Worker serves: fresh media addresses. */
export async function instagramFiles(code) {
  const item = await instagramItem(code);
  if (!item) return null;
  return {
    avatar: httpsUrl(item.user?.profile_pic_url) ?? null,
    media: instagramMedia(item).map(({ image, video }) => ({
      image,
      video: video && INSTAGRAM_CDN.test(new URL(video).hostname) ? video : null,
    })),
  };
}

async function readInstagram(url) {
  const code = instagramShortcode(url);
  const item = code && (await instagramItem(code).catch(() => null));
  if (item?.user && item.code === code) return embed(url.href, site(url, "Instagram"), instagramBlocks(item, code));
  const found = await generic(url);
  return found ? tidyInstagram(found) : null;
}

const READERS = [
  { domains: X_DOMAINS, read: (url) => X_STATUS.exec(url.pathname) && readX(url, X_STATUS.exec(url.pathname)) },
  { domains: ["tiktok.com"], read: readTikTok },
  { domains: ["youtube.com", "youtu.be", "youtube-nocookie.com"], read: readYouTube },
  { domains: ["vimeo.com"], read: readVimeo },
  { domains: ["twitch.tv"], read: readTwitch },
  { domains: ["instagram.com"], read: readInstagram },
];

// Never throws: a site that fails or refuses simply gets no preview.
export async function unfurl(url) {
  const host = hostOf(url);
  const reader = READERS.find((candidate) => onDomain(host, candidate.domains));
  try {
    const result = reader ? await reader.read(url) : null;
    // X pages carry no preview tags, so a missing or deleted post stays bare.
    if (result || reader?.domains === X_DOMAINS) return result || null;
    return await generic(url);
  } catch {
    return null;
  }
}
