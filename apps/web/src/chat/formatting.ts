import { linkSegments } from "./links.ts";

export const formatStyles = [
  "bold",
  "italic",
  "underline",
  "strikethrough",
  "spoiler",
  "quote",
  "code",
  "codeBlock",
] as const;
export type FormatStyle = (typeof formatStyles)[number];
/** Style bits shared with the native clients (shared/messages/formatting.rs). */
export const styleBits: Record<FormatStyle, number> = {
  bold: 1,
  italic: 2,
  underline: 4,
  strikethrough: 8,
  spoiler: 16,
  quote: 32,
  code: 64,
  codeBlock: 128,
};
/** Inline delimiters in the order they are tried at one position; `__` before `_`, `**` before `*`. */
export const inlineMarkers = [
  ["bold", "**"],
  ["underline", "__"],
  ["strikethrough", "~~"],
  ["spoiler", "||"],
  ["italic", "_"],
  ["italic", "*"],
  ["code", "`"],
] as const;

export type FormattedRun = { text: string } & Record<FormatStyle, boolean>;

export const formatSpace = (character: string) => /\s/.test(character);

/** Whether the line `text.slice(start, end)` is a quote: `> ` and some non-blank text. */
export function quotedLine(text: string, start: number, end: number) {
  return text.startsWith("> ", start) && end > start + 2 && /\S/.test(text.slice(start + 2, end));
}

/** Whether a fenced code block opening info string is valid (optional language tag). */
export function validFenceInfo(text: string, start: number, end: number) {
  const info = text.slice(start, end).trim();
  return /^[A-Za-z0-9_+#.-]*$/.test(info);
}

/** Whether `text.slice(start, end)` is a closing fence line: ```` ``` ```` + optional whitespace. */
export function isClosingFence(text: string, start: number, end: number) {
  if (end < start + 3 || !text.startsWith("```", start)) return false;
  for (let i = start + 3; i < end; i++) {
    if (!formatSpace(text[i])) return false;
  }
  return true;
}

/** Whether `text.slice(start, end)` is a single-line fenced block: ```` ```code``` ````. */
export function singleLineFence(text: string, start: number, end: number): [number, number] | undefined {
  let trimEnd = end;
  while (trimEnd > start && formatSpace(text[trimEnd - 1])) trimEnd--;
  if (trimEnd <= start + 6) return undefined;
  if (!text.startsWith("```", start) || text.slice(trimEnd - 3, trimEnd) !== "```") return undefined;
  const innerStart = start + 3;
  const innerEnd = trimEnd - 3;
  if (text[innerStart] === "`" || text[innerEnd - 1] === "`") return undefined;
  if (text.slice(innerStart, innerEnd).includes("\n")) return undefined;
  if (!/\S/.test(text.slice(innerStart, innerEnd))) return undefined;
  if (text.slice(innerStart, innerEnd).includes("```")) return undefined;
  return [innerStart, innerEnd];
}

/** Finds the closing line index for a fenced code block starting at `lines[index]`. */
export function fencedBlockEnd(text: string, lines: [number, number][], index: number): number | undefined {
  const [start, end] = lines[index];
  if (end < start + 3 || !text.startsWith("```", start)) return undefined;
  if (singleLineFence(text, start, end)) return index;
  if (!validFenceInfo(text, start + 3, end)) return undefined;
  for (let j = index + 1; j < lines.length; j++) {
    if (isClosingFence(text, lines[j][0], lines[j][1])) return j;
  }
  return undefined;
}

/** A small Markdown subset, as Discord writes it: **bold**, _italic_ / *italic*,
 * __underline__, ~~strikethrough~~, ||spoiler||, `code`, `> ` quoted lines, and
 * fenced ``` code blocks. Delimiters must enclose non-whitespace. Different styles
 * nest (except inside code); escapes, unmatched delimiters, snake_case identifiers
 * and URL contents stay literal.
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
    // `***`, `~~~`, `|||` and `` `` `` stay literal; underscores follow word rules.
    return marker[0] === "_" || (text[index - 1] !== marker[0] && text[index + marker.length] !== marker[0]);
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
  // Single-underscore italic delimiters may not touch a word character inside the
  // range; `*italic*` and `__underline__` work mid-word; inline `` `code` `` does
  // not span newlines and suppresses nested Markdown.
  const parse = (start: number, end: number, style: number) => {
    let plain = start;
    let index = start;
    scan: while (index < end) {
      for (const [name, marker] of inlineMarkers) {
        const bit = styleBits[name];
        const singleUnderscore = marker === "_";
        const backtick = name === "code";
        const inner = index + marker.length;
        if (
          style & bit ||
          inner >= end ||
          !markerAt(index, marker) ||
          formatSpace(text[inner]) ||
          (singleUnderscore && index > start && word(text[index - 1])) ||
          (singleUnderscore && text[inner] === "_")
        )
          continue;
        for (let close = inner + 1; close + marker.length <= end; close++) {
          if (backtick && text[close - 1] === "\n") break;
          if (!markerAt(close, marker) || formatSpace(text[close - 1])) continue;
          if (singleUnderscore && close + marker.length < end && word(text[close + marker.length])) continue;
          if (singleUnderscore && text[close - 1] === "_") continue;
          if (marker === "__" && text[close + marker.length] === "_") continue;
          append(plain, index, style);
          if (backtick) append(inner, close, style | bit);
          else parse(inner, close, style | bit);
          index = close + marker.length;
          plain = index;
          continue scan;
        }
      }
      index++;
    }
    append(plain, end, style);
  };
  const rawLines: [number, number][] = [];
  for (let start = 0; ;) {
    const newline = text.indexOf("\n", start);
    const end = newline < 0 ? text.length : newline;
    rawLines.push([start, end]);
    if (newline < 0) break;
    start = end + 1;
  }
  const isFence = Array<boolean>(rawLines.length).fill(false);
  for (let idx = 0; idx < rawLines.length;) {
    const close = fencedBlockEnd(text, rawLines, idx);
    if (close !== undefined) {
      for (let k = idx; k <= close; k++) isFence[k] = true;
      idx = close + 1;
    } else idx++;
  }
  const lines: [number, number, boolean, boolean][] = rawLines.map(([s, e], i) => [
    s,
    e,
    !isFence[i] && quotedLine(text, s, e),
    isFence[i],
  ]);
  for (let index = 0; index < lines.length;) {
    const [start, end, quoted, fence] = lines[index];
    if (fence) {
      const close = fencedBlockEnd(text, rawLines, index) ?? index;
      append(start, lines[close][1], styleBits.codeBlock);
      index = close + 1;
    } else if (quoted) {
      parse(start + 2, end, styleBits.quote);
      index++;
    } else {
      let last = index;
      while (last + 1 < lines.length && !lines[last + 1][2] && !lines[last + 1][3]) last++;
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
