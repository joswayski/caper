/** KLIPY GIFs and stickers. KLIPY requires its search, share and media
 * requests to go straight from the browser, so only the app key and the
 * account's customer id come from our API. Rules: docs/media.md and
 * shared/klipy/cases.json. */

export type KlipyKind = "gif" | "sticker";

/** A message's `content.gif`, holding KLIPY's media URLs unchanged. */
export interface ChatGif {
  provider: "klipy";
  kind: KlipyKind;
  slug: string;
  title: string;
  width: number;
  height: number;
  gif?: string;
  webp?: string;
  mp4?: string;
  still?: string;
}

export interface KlipyConfig {
  appKey: string;
  customerId: string;
}

export type KlipyItem =
  | {
      type: "item";
      slug: string;
      title: string;
      tile: { url: string; still?: string; width: number; height: number };
      gif: ChatGif;
    }
  | { type: "ad"; html: string; width: number | null; height: number | null };

export interface KlipyPage {
  items: KlipyItem[];
  hasNext: boolean;
}

const SLUG = /^[A-Za-z0-9_-]{1,200}$/;
// oxlint-disable-next-line no-control-regex -- Titles never carry control characters.
const CONTROLS = /[\u0000-\u001f\u007f-\u009f]/gu;
// oxlint-disable-next-line no-control-regex -- A separate, stateless test.
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/u;

/** KLIPY media only: HTTPS on klipy.com or a subdomain, no credentials, port or fragment. */
export function klipyMediaUrl(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 2048 || value.includes("#")) return false;
  try {
    const url = new URL(value);
    return (
      url.protocol === "https:" &&
      !url.username &&
      !url.password &&
      // An explicit default port normalizes away, as in the API's URL parser.
      !url.port &&
      (url.hostname === "klipy.com" || url.hostname.endsWith(".klipy.com"))
    );
  } catch {
    return false;
  }
}

const edge = (value: unknown): value is number =>
  typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 4096;

/** The GIF to render, or undefined to show the text alone. */
export function gifOf(content: { gif?: unknown }): ChatGif | undefined {
  const gif = content.gif as Partial<ChatGif> | undefined;
  if (!gif || typeof gif !== "object") return undefined;
  const urls = [gif.gif, gif.webp, gif.mp4, gif.still];
  return gif.provider === "klipy" &&
    (gif.kind === "gif" || gif.kind === "sticker") &&
    typeof gif.slug === "string" &&
    SLUG.test(gif.slug) &&
    (gif.title === undefined ||
      (typeof gif.title === "string" && Array.from(gif.title).length <= 200 && !CONTROL.test(gif.title))) &&
    edge(gif.width) &&
    edge(gif.height) &&
    (gif.gif !== undefined || gif.webp !== undefined) &&
    urls.every((url) => url === undefined || klipyMediaUrl(url))
    ? (gif as ChatGif)
    : undefined;
}

export function gifLabel(gif: Pick<ChatGif, "kind" | "title">) {
  return gif.title || (gif.kind === "sticker" ? "Sticker" : "GIF");
}

type Format = { url?: unknown; width?: unknown; height?: unknown };
type Size = Record<string, Format | undefined>;

const url = (format: Format | undefined) => (klipyMediaUrl(format?.url) ? format.url : undefined);

function parseItem(raw: Record<string, unknown>, kind: KlipyKind): KlipyItem | undefined {
  if (raw.type === "ad") {
    if (typeof raw.content !== "string" || !raw.content) return undefined;
    const size = (value: unknown) => (typeof value === "number" && value > 0 ? value : null);
    return { type: "ad", html: raw.content, width: size(raw.width), height: size(raw.height) };
  }
  if (typeof raw.slug !== "string" || !SLUG.test(raw.slug)) return undefined;
  const file = (raw.file ?? {}) as Record<string, Size | undefined>;
  const sized = (format: Format | undefined) =>
    url(format) && edge(format?.width) && edge(format?.height) ? format : undefined;
  const send = file.md ?? file.hd ?? file.sm;
  const tileSize = file.sm ?? file.xs ?? file.md ?? file.hd;
  const dimensions = send && (sized(send.gif) ?? sized(send.webp));
  const tileFormat = tileSize && (sized(tileSize.webp) ?? sized(tileSize.gif));
  if (!send || !dimensions || !tileFormat) return undefined;
  const title = Array.from(typeof raw.title === "string" ? raw.title.replace(CONTROLS, "") : "")
    .slice(0, 200)
    .join("");
  const gif: ChatGif = {
    provider: "klipy",
    kind,
    slug: raw.slug,
    title,
    width: dimensions.width as number,
    height: dimensions.height as number,
  };
  const formats = {
    gif: url(send.gif),
    webp: url(send.webp),
    mp4: url(send.mp4),
    still: url(send.jpg) ?? url(send.png),
  };
  for (const [key, value] of Object.entries(formats)) if (value) gif[key as "gif"] = value;
  if (!gif.gif && !gif.webp) return undefined;
  const still = url(tileSize.jpg) ?? url(tileSize.png);
  return {
    type: "item",
    slug: raw.slug,
    title,
    tile: {
      url: tileFormat.url as string,
      ...(still ? { still } : {}),
      width: tileFormat.width as number,
      height: tileFormat.height as number,
    },
    gif,
  };
}

/** KLIPY's list response as picker items, in KLIPY's order. */
export function parseKlipy(response: unknown, kind: KlipyKind): KlipyPage {
  const data = (response as { data?: { data?: unknown; has_next?: unknown } } | null)?.data;
  const rows = Array.isArray(data?.data) ? data.data : [];
  return {
    items: rows.flatMap((row) => {
      const item = row && typeof row === "object" ? parseItem(row as Record<string, unknown>, kind) : undefined;
      return item ? [item] : [];
    }),
    hasNext: data?.has_next === true,
  };
}

let config: Promise<KlipyConfig | null> | undefined;

/** The app key and customer id, or null when GIFs are off or signed out. */
export function klipyConfig(): Promise<KlipyConfig | null> {
  config ??= fetch("/api/klipy", { credentials: "same-origin" })
    .then(async (response) => {
      if (!response.ok) return null;
      const value = (await response.json()) as Partial<KlipyConfig>;
      return typeof value.appKey === "string" &&
        /^[A-Za-z0-9_-]{1,200}$/.test(value.appKey) &&
        typeof value.customerId === "string"
        ? (value as KlipyConfig)
        : null;
    })
    .catch(() => null)
    .then((value) => {
      // Retry later after a failure or sign-in rather than hiding GIFs for the tab's lifetime.
      if (!value) config = undefined;
      return value;
    });
  return config;
}

const BASE = "https://api.klipy.com/api/v1";
const path = (kind: KlipyKind) => (kind === "sticker" ? "stickers" : "gifs");

function region() {
  const locale = new Intl.Locale(navigator.language || "en-US").maximize();
  return locale.region?.toLowerCase();
}

/** Ads are requested only from touch layouts (KLIPY serves them to mobile
 * browsers), sized to the grid. */
export async function klipyPage(
  key: KlipyConfig,
  kind: KlipyKind,
  query: string,
  page: number,
  ads: { width: number } | undefined,
  signal: AbortSignal,
): Promise<KlipyPage> {
  const params = new URLSearchParams({
    page: String(page),
    per_page: "24",
    customer_id: key.customerId,
    content_filter: "off",
  });
  if (query) params.set("q", query);
  const area = region();
  if (area) params.set("locale", area);
  if (ads) {
    params.set("ad-min-width", "50");
    params.set("ad-max-width", String(Math.max(50, Math.floor(ads.width))));
    params.set("ad-min-height", "50");
    params.set("ad-max-height", "250");
    params.set("ad-device-w", String(Math.round(screen.width * devicePixelRatio)));
    params.set("ad-device-h", String(Math.round(screen.height * devicePixelRatio)));
    params.set("ad-pxratio", String(devicePixelRatio));
    params.set("ad-language", (navigator.language || "en").slice(0, 2).toUpperCase());
  }
  const response = await fetch(
    `${BASE}/${encodeURIComponent(key.appKey)}/${path(kind)}/${query ? "search" : "trending"}?${params}`,
    { signal, credentials: "omit", referrerPolicy: "origin" },
  );
  if (!response.ok) throw new Error("KLIPY request failed");
  return parseKlipy(await response.json(), kind);
}

/** Best effort: tells KLIPY what was shared, for ranking and recents. */
export function shareKlipy(key: KlipyConfig, gif: ChatGif, query: string) {
  void fetch(`${BASE}/${encodeURIComponent(key.appKey)}/${path(gif.kind)}/share/${encodeURIComponent(gif.slug)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ customer_id: key.customerId, q: query }),
    credentials: "omit",
    referrerPolicy: "origin",
    keepalive: true,
  }).catch(() => undefined);
}
