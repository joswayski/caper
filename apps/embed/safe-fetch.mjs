// Outbound fetches for link previews. Every URL here came from a message, so
// it is treated as hostile: http(s) on the default ports only, no credentials,
// no IP literals or internal names, every redirect hop re-checked, and short
// timeouts with a byte cap on what is read. The Worker itself runs on
// Cloudflare, outside the AWS network, so even a missed case cannot reach
// Caper's cluster, databases or the instance metadata service.

export const USER_AGENT = "Mozilla/5.0 (compatible; Caperbot/1.0; +https://caper.chat)";
const MAX_REDIRECTS = 5;
const TIMEOUT_MS = 6000;
// Names that only resolve inside a private network.
const INTERNAL_SUFFIXES = [".local", ".localhost", ".internal", ".intranet", ".lan", ".home.arpa", ".corp"];

export class FetchRefused extends Error {}

// Drops a body we will not read, without waiting on the network.
export function discard(response) {
  response?.body?.cancel().catch(() => {});
}

function isIpLiteral(hostname) {
  // The URL parser already normalised IPv4 shorthand (0x7f.1, 2130706433) to
  // dotted form and wraps IPv6 in brackets.
  return hostname.startsWith("[") || /^\d{1,3}(\.\d{1,3}){3}$/.test(hostname);
}

// Returns the parsed URL when it is safe to fetch, otherwise null.
export function fetchableUrl(value, base) {
  let url;
  try {
    url = new URL(value, base);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (url.username || url.password) return null;
  if (url.port && url.port !== "80" && url.port !== "443") return null;
  const host = url.hostname.toLowerCase().replace(/\.$/, "");
  if (!host.includes(".") || isIpLiteral(host)) return null;
  if (INTERNAL_SUFFIXES.some((suffix) => host.endsWith(suffix))) return null;
  url.hash = "";
  return url;
}

// fetch() that follows at most MAX_REDIRECTS, checking every hop. Returns the
// final response and the URL it came from.
export async function safeFetch(value, { headers = {}, timeout = TIMEOUT_MS } = {}) {
  let url = fetchableUrl(value);
  if (!url) throw new FetchRefused("url not allowed");
  const signal = AbortSignal.timeout(timeout);
  for (let hop = 0; ; hop++) {
    const response = await fetch(url, {
      headers: { "user-agent": USER_AGENT, ...headers },
      redirect: "manual",
      signal,
    });
    if (response.status < 300 || response.status > 399 || !response.headers.has("location")) {
      return { response, url };
    }
    discard(response);
    if (hop >= MAX_REDIRECTS) throw new FetchRefused("too many redirects");
    const next = fetchableUrl(response.headers.get("location"), url);
    if (!next) throw new FetchRefused("redirect not allowed");
    url = next;
  }
}

// Reads at most `limit` bytes of text, stopping early once `until` appears
// (for HTML, the end of <head>, which holds every tag a preview needs).
export async function readText(response, limit, until) {
  if (!response.body) return "";
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = "";
  let bytes = 0;
  try {
    while (bytes < limit) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      text += decoder.decode(value, { stream: true });
      if (until && text.includes(until)) break;
    }
  } finally {
    reader.cancel().catch(() => {});
  }
  return text.slice(0, limit);
}

export function contentType(response) {
  return (response.headers.get("content-type") ?? "").split(";")[0].trim().toLowerCase();
}
