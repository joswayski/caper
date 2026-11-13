import { linkSegments } from "./links.ts";

export const formatStyles = ["bold", "italic", "underline", "strikethrough", "spoiler", "quote"] as const;
export type FormatStyle = (typeof formatStyles)[number];
/** Style bits shared with the native clients (shared/messages/formatting.rs). */
export const styleBits: Record<FormatStyle, number> = {
  bold: 1,
  italic: 2,
  underline: 4,
  strikethrough: 8,
  spoiler: 16,
  quote: 32,
};
/** Inline delimiters in the order they are tried at one position; `__` before `_`. */
export const inlineMarkers = [
  ["bold", "**"],
  ["underline", "__"],
  ["strikethrough", "~~"],
  ["spoiler", "||"],
  ["italic", "_"],
] as const;

export type FormattedRun = { text: string } & Record<FormatStyle, boolean>;

export const formatSpace = (character: string) => /\s/.test(character);

/** Whether the line `text.slice(start, end)` is a quote: `> ` and some non-blank text. */
export function quotedLine(text: string, start: number, end: number) {
  return text.startsWith("> ", start) && end > start + 2 && /\S/.test(text.slice(start + 2, end));
}

/** A small Markdown subset, as Discord writes it: **bold**, _italic_, __underline__,
 * ~~strikethrough~~, ||spoiler|| and `> ` quoted lines. Delimiters must enclose
 * non-whitespace. Different styles nest; escapes, unmatched delimiters,
 * identifiers and URL contents stay literal.
 * Keep native implementations aligned with shared/messages/formatting-cases.json.
 */
export function formattedRuns(text: string): FormattedRun[] {
  return formattedMessage(text).runs;
}

/** Visible UTF-16 characters mapped back to the stored message, for composer selections. */
export function formattedMessage(text: string) {
  const runs: FormattedRun[] = [];
  const styles: number[] = [];
  const sourceIndices: number[] = [];
  const links: [number, number][] = [];
  let offset = 0;
  for (const part of linkSegments(text)) {
    if (part.href) links.push([offset, offset + part.text.length]);
    offset += part.text.length;
  }
  const word = (character: string | undefined) => !!character && /[A-Za-z0-9_@]/.test(character);
  const markerAt = (index: number, marker: string) => {
    if (!text.startsWith(marker, index) || links.some(([start, end]) => index >= start && index < end)) return false;
    let slashes = 0;
    for (let i = index - 1; i >= 0 && text[i] === "\\"; i--) slashes++;
    if (slashes % 2) return false;
    // `***`, `~~~` and `|||` stay literal; underscores follow word rules.
    return marker.length !== 2 || marker[0] === "_" || (text[index - 1] !== marker[0] && text[index + 2] !== marker[0]);
  };
  const append = (start: number, end: number, style: number) => {
    if (start >= end) return;
    const value = text.slice(start, end);
    for (let index = start; index < end; index++) {
      sourceIndices.push(index);
      styles.push(style);
    }
    const last = runs.at(-1);
    if (last && formatStyles.every((name) => last[name] === !!(style & styleBits[name]))) last.text += value;
    else
      runs.push({
        text: value,
        ...(Object.fromEntries(formatStyles.map((name) => [name, !!(style & styleBits[name])])) as Record<
          FormatStyle,
          boolean
        >),
      });
  };
  // Underscore delimiters may not touch a word character inside the range; a
  // delimiter just outside it belongs to an enclosing style.
  const parse = (start: number, end: number, style: number) => {
    let plain = start;
    let index = start;
    scan: while (index < end) {
      for (const [name, marker] of inlineMarkers) {
        const bit = styleBits[name];
        const underscore = marker[0] === "_";
        const inner = index + marker.length;
        if (
          style & bit ||
          inner >= end ||
          !markerAt(index, marker) ||
          formatSpace(text[inner]) ||
          (underscore && index > start && word(text[index - 1])) ||
          (marker === "_" && text[inner] === "_")
        )
          continue;
        for (let close = inner + 1; close + marker.length <= end; close++) {
          if (!markerAt(close, marker) || formatSpace(text[close - 1])) continue;
          if (underscore && close + marker.length < end && word(text[close + marker.length])) continue;
          if (marker === "_" && text[close - 1] === "_") continue;
          append(plain, index, style);
          parse(inner, close, style | bit);
          index = close + marker.length;
          plain = index;
          continue scan;
        }
      }
      index++;
    }
    append(plain, end, style);
  };
  // Unquoted lines parse together, so emphasis may span them; each quoted line parses alone.
  const lines: [number, number, boolean][] = [];
  for (let start = 0; ;) {
    const newline = text.indexOf("\n", start);
    const end = newline < 0 ? text.length : newline;
    lines.push([start, end, quotedLine(text, start, end)]);
    if (newline < 0) break;
    start = end + 1;
  }
  for (let index = 0; index < lines.length;) {
    const [start, end, quoted] = lines[index];
    if (quoted) {
      parse(start + 2, end, styleBits.quote);
      index++;
    } else {
      let last = index;
      while (last + 1 < lines.length && !lines[last + 1][2]) last++;
      parse(start, lines[last][1], 0);
      index = last + 1;
    }
    const [, lineEnd, lineQuoted] = lines[index - 1];
    if (index < lines.length) append(lineEnd, lineEnd + 1, lineQuoted && lines[index][2] ? styleBits.quote : 0);
  }
  return { runs, styles, sourceIndices };
}

/** A whole link may be styled, but a delimiter must never split its address. */
export function splitsLink(text: string, start: number, end: number) {
  let offset = 0;
  for (const part of linkSegments(text)) {
    const limit = offset + part.text.length;
    if (part.href && ((start > offset && start < limit) || (end > offset && end < limit))) return true;
    offset = limit;
  }
  return false;
}
