// The page native apps open in a web view when Play is tapped: nothing but the
// provider's own player in an iframe. YouTube refuses to play without a real
// website as referrer and Twitch without a `parent` domain, so a bare web view
// cannot load them directly. The page itself runs no script; its CSP allows
// only the three players, and nothing may frame it.

const PLAYERS = {
  youtube: {
    id: /^[A-Za-z0-9_-]{11}$/,
    src: (id, start) =>
      `https://www.youtube-nocookie.com/embed/${id}?autoplay=1&playsinline=1${start ? `&start=${start}` : ""}`,
  },
  vimeo: { id: /^\d{1,12}$/, src: (id) => `https://player.vimeo.com/video/${id}?autoplay=1` },
  twitch: {
    id: /^(clip:[A-Za-z0-9_-]{3,100}|video:\d{1,15}|channel:[a-z0-9_]{3,25})$/,
    src: (id, _start, host) => {
      const [kind, value] = id.split(":");
      const parent = `parent=${encodeURIComponent(host)}&autoplay=true`;
      if (kind === "clip") return `https://clips.twitch.tv/embed?clip=${value}&${parent}`;
      if (kind === "video") return `https://player.twitch.tv/?video=v${value}&${parent}`;
      return `https://player.twitch.tv/?channel=${value}&${parent}`;
    },
  },
};

export function playPath(block) {
  const start = Number.isInteger(block.start) && block.start > 0 ? `?start=${block.start}` : "";
  return `/play/${block.provider}/${encodeURIComponent(block.id)}${start}`;
}

const escape = (value) => value.replace(/[&<>"']/g, (character) => `&#${character.charCodeAt(0)};`);

export function playerPage(url) {
  const [, , provider, raw] = url.pathname.split("/");
  const player = PLAYERS[provider];
  let id;
  try {
    id = decodeURIComponent(raw ?? "");
  } catch {
    id = "";
  }
  const start = Number(url.searchParams.get("start") ?? 0);
  if (!player || !player.id.test(id) || !Number.isInteger(start) || start < 0) {
    return new Response("Not found", {
      status: 404,
      headers: { "content-type": "text/plain", "cache-control": "no-store" },
    });
  }
  const src = player.src(id, start, url.hostname);
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Player</title>
<style>html,body{margin:0;height:100%;background:#000}iframe{display:block;border:0;width:100%;height:100%}</style>
</head><body><iframe src="${escape(src)}" title="Video player" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowfullscreen referrerpolicy="strict-origin-when-cross-origin"></iframe></body></html>`;
  return new Response(html, {
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "public, max-age=86400",
      "content-security-policy":
        "default-src 'none'; style-src 'unsafe-inline'; frame-src https://www.youtube-nocookie.com https://player.vimeo.com https://player.twitch.tv https://clips.twitch.tv; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
      "referrer-policy": "strict-origin-when-cross-origin",
      "x-content-type-options": "nosniff",
      "x-frame-options": "DENY",
    },
  });
}
