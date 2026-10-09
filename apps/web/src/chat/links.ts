/**
 * Plain-text link detection for message text, shared by every client.
 *
 * Messages stay plain text on the wire (`content.text`); links are found at
 * render time, the same way mentions are. The rules follow GitHub Flavored
 * Markdown's autolink literals (so a later Markdown renderer can reuse this
 * function for bare URLs without changing how old messages look), with
 * quotes and unbalanced `]` also treated as trailing punctuation.
 *
 * Every client implements this exact algorithm and runs the cases in
 * shared/messages/link-cases.json. Change them together.
 */

export type LinkSegment = { text: string; href?: undefined } | { text: string; href: string };

/** Unicode White_Space; links end at the first of these or `<`. */
const whitespace = /[\t-\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/;
/** A link may start at the beginning, after whitespace, or after one of these. */
const openers = new Set(["(", "[", "{", "<", '"', "'", "*", "_", "~"]);
const trailing = new Set(["?", "!", ".", ",", ":", ";", "*", "_", "~", '"', "'", ">"]);
const hostCharacter = /[A-Za-z0-9_.-]/;

function startsAt(text: string, index: number) {
  const rest = text.slice(index, index + 8).toLowerCase();
  if (rest.startsWith("https://")) return 8;
  if (rest.startsWith("http://")) return 7;
  if (rest.startsWith("www.")) return 0;
  return -1;
}

function count(text: string, character: string) {
  let total = 0;
  for (const value of text) if (value === character) total++;
  return total;
}

/** Drops trailing punctuation, unbalanced closers and a trailing `&entity;`. */
function trim(candidate: string) {
  let value = candidate;
  for (;;) {
    const last = value.at(-1);
    if (last === undefined) return value;
    if (trailing.has(last)) {
      const entity = last === ";" ? /&[A-Za-z0-9]+;$/.exec(value) : null;
      value = entity ? value.slice(0, entity.index) : value.slice(0, -1);
      continue;
    }
    if (last === ")" && count(value, ")") > count(value, "(")) {
      value = value.slice(0, -1);
      continue;
    }
    if (last === "]" && count(value, "]") > count(value, "[")) {
      value = value.slice(0, -1);
      continue;
    }
    return value;
  }
}

/** At least two non-empty labels (three for `www.`), no `_` in the last two. */
function validHost(host: string, www: boolean) {
  const labels = host.split(".");
  if (labels.length < (www ? 3 : 2) || labels.some((label) => !label)) return false;
  return !labels.slice(-2).some((label) => label.includes("_"));
}

/** Splits plain text into text runs and `http(s)://` / `www.` links. */
export function linkSegments(text: string): LinkSegment[] {
  const segments: LinkSegment[] = [];
  let plainStart = 0;
  let index = 0;
  while (index < text.length) {
    const previous = index > 0 ? text[index - 1] : undefined;
    const scheme =
      previous === undefined || whitespace.test(previous) || openers.has(previous) ? startsAt(text, index) : -1;
    if (scheme < 0) {
      index++;
      continue;
    }
    let end = index;
    while (end < text.length && !whitespace.test(text[end]) && text[end] !== "<") end++;
    const candidate = trim(text.slice(index, end));
    let hostEnd = scheme;
    while (hostEnd < candidate.length && hostCharacter.test(candidate[hostEnd])) hostEnd++;
    if (!validHost(candidate.slice(scheme, hostEnd), scheme === 0)) {
      index++;
      continue;
    }
    if (index > plainStart) segments.push({ text: text.slice(plainStart, index) });
    const href =
      scheme === 0 ? `https://${candidate}` : candidate.slice(0, scheme).toLowerCase() + candidate.slice(scheme);
    segments.push({ text: candidate, href });
    index += candidate.length;
    plainStart = index;
  }
  if (plainStart < text.length) segments.push({ text: text.slice(plainStart) });
  return segments;
}
