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
  const runs: FormattedRun[] = [];
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
  const append = (value: string, bold: boolean, italic: boolean) => {
    if (!value) return;
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
        append(text.slice(plain, index), bold, italic);
        parse(inner, close, bold || marker === "**", italic || marker === "_");
        index = close + marker.length - 1;
        plain = index + 1;
        break;
      }
    }
    append(text.slice(plain, end), bold, italic);
  };
  parse(0, text.length, false, false);
  return runs;
}

/** Toggle markers around a selection, retaining its content and selection. */
export function toggleFormat(text: string, start: number, end: number, marker: "**" | "_") {
  if (start !== end) {
    while (start < end && /\s/.test(text[start])) start++;
    while (end > start && /\s/.test(text[end - 1])) end--;
    if (start === end) return { text, start, end };
  }
  const width = marker.length;
  if (start >= width && text.slice(start - width, start) === marker && text.slice(end, end + width) === marker) {
    return {
      text: text.slice(0, start - width) + text.slice(start, end) + text.slice(end + width),
      start: start - width,
      end: end - width,
    };
  }
  let selected = text.slice(start, end);
  const runs = formattedRuns(selected);
  const hasStyle = (run: FormattedRun) => (marker === "**" ? run.bold : run.italic);
  if (
    selected.length >= width * 2 &&
    selected.startsWith(marker) &&
    selected.endsWith(marker) &&
    runs.every(hasStyle)
  ) {
    return {
      text: text.slice(0, start) + selected.slice(width, -width) + text.slice(end),
      start,
      end: end - width * 2,
    };
  }
  // Applying a style to a mixed selection must not nest its own delimiters.
  if (runs.some(hasStyle)) {
    const remaining: { text: string; styled: boolean }[] = [];
    for (const run of runs) {
      const styled = marker === "**" ? run.italic : run.bold;
      const last = remaining.at(-1);
      if (last?.styled === styled) last.text += run.text;
      else remaining.push({ text: run.text, styled });
    }
    const other = marker === "**" ? "_" : "**";
    selected = remaining.map((run) => (run.styled ? other + run.text + other : run.text)).join("");
  }
  return {
    text: text.slice(0, start) + marker + selected + marker + text.slice(end),
    start: start + width,
    end: start + width + selected.length,
  };
}
