/**
 * Links to one message: `https://caper.chat/m/<channel>/<message>` (Copy link)
 * and `caper://m/<channel>/<message>` for the apps. The same rule as
 * `shared/messages/message_links.rs`; every client runs
 * `shared/messages/message-link-cases.json`. Only caper.chat hosts count, so
 * another site's `/m/...` path stays an ordinary link.
 */
export const MESSAGE_LINK_ORIGIN = "https://caper.chat";

const HOSTS = new Set(["caper.chat", "www.caper.chat", "staging.caper.chat"]);
const IDS = /^([A-Za-z0-9]{12})\/([A-Za-z0-9]{15})\/?(?:[?#][\s\S]*)?$/;

export interface MessageLink {
  channel: string;
  message: string;
}

function ids(path: string): MessageLink | undefined {
  const match = IDS.exec(path);
  return match ? { channel: match[1], message: match[2] } : undefined;
}

/** The message a link opens, or undefined for anything else. */
export function messageLink(href: string): MessageLink | undefined {
  const value = href.trim();
  if (/^caper:\/\/m\//i.test(value)) return ids(value.slice(10));
  const scheme = /^https?:\/\//i.exec(value);
  if (!scheme) return undefined;
  const rest = value.slice(scheme[0].length);
  const slash = rest.indexOf("/");
  if (slash < 0 || !HOSTS.has(rest.slice(0, slash).toLowerCase())) return undefined;
  const path = rest.slice(slash);
  if (!/^\/m\//i.test(path)) return undefined;
  return ids(path.slice(3));
}

/** The link Copy link puts on the clipboard. */
export function messageLinkUrl(channel: string, message: string) {
  return `${MESSAGE_LINK_ORIGIN}/m/${channel}/${message}`;
}
