// Embed content, version 1. An embed is an ordered list of blocks that every
// client draws natively; clients skip block types they do not know. See
// docs/media.md "Link embeds" for the contract each client implements.
//
// { version: 1, url, site: { name, domain }, blocks: [...] }
//   author  { name, handle?, url?, icon? }
//   title   { text, url? }
//   text    { text }
//   images  { items: [{ url, width?, height?, alt? }], thumbnail? }  1-4 items
//   video   { url, poster?, width?, height?, durationMs?, gif? }      direct MP4/WebM
//   player  { provider, id, poster?, width?, height?, start? }       YouTube, Vimeo, Twitch
//   footer  { text, timestamp? }

export const VERSION = 1;
export const LIMITS = {
  name: 128,
  handle: 64,
  title: 256,
  text: 4000,
  description: 600,
  footer: 256,
  alt: 300,
  images: 4,
};

// Plain text only: no control characters except newlines, collapsed runs of
// blank lines, and a hard length cap on code points.
export function clean(value, limit) {
  if (typeof value !== "string") return null;
  const text = value
    .replace(/\r\n?/g, "\n")
    // oxlint-disable-next-line no-control-regex -- stripping them is the point
    .replace(/[\u0000-\u0009\u000b-\u001f\u007f-\u009f​-‏‪-‮⁦-⁩]/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  if (!text) return null;
  const points = [...text];
  return points.length > limit
    ? `${points
        .slice(0, limit - 1)
        .join("")
        .trimEnd()}…`
    : text;
}

export function dimension(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 && number <= 16384 ? number : undefined;
}

export function milliseconds(value) {
  const number = Math.round(Number(value));
  return Number.isFinite(number) && number > 0 && number <= 24 * 3600 * 1000 ? number : undefined;
}

export function httpsUrl(value, base) {
  try {
    const url = new URL(value, base);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

// Drops empty fields and blocks so clients never see half-filled ones.
function compact(object) {
  return Object.fromEntries(Object.entries(object).filter(([, value]) => value !== undefined && value !== null));
}

export function embed(url, site, blocks) {
  const kept = blocks.filter(Boolean).map(compact);
  return kept.length ? { version: VERSION, url, site, blocks: kept } : null;
}

export const author = (name, extra = {}) => {
  const text = clean(name, LIMITS.name);
  return text ? { type: "author", name: text, ...extra, handle: clean(extra.handle, LIMITS.handle) } : null;
};
export const title = (text, url) => {
  const value = clean(text, LIMITS.title);
  return value ? { type: "title", text: value, url } : null;
};
export const text = (value, limit = LIMITS.text) => {
  const cleaned = clean(value, limit);
  return cleaned ? { type: "text", text: cleaned } : null;
};
export const images = (items, thumbnail) => {
  const kept = items
    .filter((item) => item?.url)
    .slice(0, LIMITS.images)
    .map((item) => compact({ ...item, alt: clean(item.alt, LIMITS.alt) }));
  return kept.length ? { type: "images", items: kept, thumbnail: thumbnail || undefined } : null;
};
export const footer = (value, timestamp) => {
  const cleaned = clean(value, LIMITS.footer);
  const time = timestamp && !Number.isNaN(Date.parse(timestamp)) ? new Date(timestamp).toISOString() : undefined;
  return cleaned || time ? { type: "footer", text: cleaned ?? undefined, timestamp: time } : null;
};

export function compactNumber(value) {
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) return null;
  if (number < 1000) return String(Math.round(number));
  for (const [size, suffix] of [
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ]) {
    if (number >= size) {
      const scaled = number / size;
      return `${scaled >= 100 ? Math.round(scaled) : Number(scaled.toFixed(1))}${suffix}`;
    }
  }
  return null;
}
