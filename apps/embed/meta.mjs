// Reads OpenGraph, Twitter Card and plain <title>/<meta> tags out of the start
// of an HTML document. Values come back as decoded plain text; nothing from the
// page is ever rendered as markup.

const NAMED = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ", "#39": "'" };

export function decodeEntities(text) {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+|#39);/gi, (match, entity) => {
    const lower = entity.toLowerCase();
    if (lower in NAMED) return NAMED[lower];
    const code = lower.startsWith("#x")
      ? parseInt(lower.slice(2), 16)
      : lower.startsWith("#")
        ? Number(lower.slice(1))
        : NaN;
    if (!Number.isInteger(code) || code < 1 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) return match;
    return String.fromCodePoint(code);
  });
}

function attributes(tag) {
  const result = {};
  for (const [, name, , double, single, bare] of tag.matchAll(
    /([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*(=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g,
  )) {
    result[name.toLowerCase()] = decodeEntities(double ?? single ?? bare ?? "");
  }
  return result;
}

// Returns { meta: Map<lowercased property/name, first value>, title, oembed }.
// Repeated og:image tags keep their order in meta.get("og:image:all").
export function parseHead(html) {
  const head = html.slice(0, Math.max(html.search(/<\/head\s*>/i), 0) || html.length);
  const meta = new Map();
  const images = [];
  for (const [tag] of head.matchAll(/<meta\b[^>]*>/gi)) {
    const attrs = attributes(tag.slice(5, -1));
    const key = (attrs.property ?? attrs.name ?? attrs.itemprop ?? "").toLowerCase();
    const value = attrs.content;
    if (!key || value === undefined) continue;
    if (key === "og:image" || key === "og:image:url" || key === "og:image:secure_url") images.push(value);
    if (!meta.has(key)) meta.set(key, value);
  }
  meta.set("og:image:all", [...new Set(images)]);
  const title = /<title\b[^>]*>([\s\S]*?)<\/title\s*>/i.exec(head)?.[1];
  let oembed = null;
  for (const [tag] of head.matchAll(/<link\b[^>]*>/gi)) {
    const attrs = attributes(tag.slice(5, -1));
    if (
      attrs.rel?.toLowerCase() === "alternate" &&
      attrs.type?.toLowerCase() === "application/json+oembed" &&
      attrs.href
    ) {
      oembed = attrs.href;
      break;
    }
  }
  return { meta, title: title === undefined ? null : decodeEntities(title.replace(/\s+/g, " ").trim()), oembed };
}
