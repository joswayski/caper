import { linkSegments } from "./links.ts";

export interface FormattedRun {
  text: string;
  bold: boolean;
  italic: boolean;
}

/** The first, deliberately small Markdown subset: **bold** and _italic_.
 * Delimiters must enclose non-whitespace. Different styles
 * nest; escapes, unmatched delimiters, identifiers and URL contents stay literal.
 * Keep native implementations aligned with shared/messages/formatting-cases.json.
 */
export function formattedRuns(text: string): FormattedRun[] {
  return formattedMessage(text).runs;
}

/** Visible UTF-16 characters mapped back to the stored message, for composer selections. */
export function formattedMessage(text: string) {
  const runs: FormattedRun[] = [];
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
    return marker !== "**" || (text[index - 1] !== "*" && text[index + 2] !== "*");
  };
  const append = (start: number, end: number, bold: boolean, italic: boolean) => {
    if (start === end) return;
    const value = text.slice(start, end);
    for (let index = start; index < end; index++) sourceIndices.push(index);
    const last = runs.at(-1);
    if (last?.bold === bold && last.italic === italic) last.text += value;
    else runs.push({ text: value, bold, italic });
  };
  const parse = (start: number, end: number, bold: boolean, italic: boolean) => {
    let plain = start;
    for (let index = start; index < end; index++) {
      const marker = !bold && markerAt(index, "**") ? "**" : !italic && markerAt(index, "_") ? "_" : undefined;
      if (!marker || (marker === "_" && word(text[index - 1]))) continue;
      const inner = index + marker.length;
      if (inner >= end || /\s/.test(text[inner])) continue;
      for (let close = inner; close + marker.length <= end; close++) {
        if (close === inner || !markerAt(close, marker) || /\s/.test(text[close - 1])) continue;
        if (marker === "_" && word(text[close + 1])) continue;
        append(plain, index, bold, italic);
        parse(inner, close, bold || marker === "**", italic || marker === "_");
        index = close + marker.length - 1;
        plain = index + 1;
        break;
      }
    }
    append(plain, end, bold, italic);
  };
  parse(0, text.length, false, false);
  return { runs, sourceIndices };
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
