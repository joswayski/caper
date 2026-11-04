import { Schema, type Node } from "prosemirror-model";
import { TextSelection, type Command, type Transaction } from "prosemirror-state";
import { toggleMark } from "prosemirror-commands";
import { formattedMessage, formattedRuns, splitsLink, type FormattedRun } from "./formatting.ts";
import { linkSegments } from "./links.ts";

// One text block keeps newlines, emoji and mention offsets identical to plain text.
export const composerSchema = new Schema({
  nodes: { doc: { content: "text*", whitespace: "pre" }, text: {} },
  marks: {
    bold: { toDOM: () => ["strong", 0], parseDOM: [{ tag: "strong" }, { tag: "b" }] },
    italic: { toDOM: () => ["em", 0], parseDOM: [{ tag: "em" }, { tag: "i" }] },
  },
});

export function composerDocument(source: string) {
  return composerSchema.node(
    "doc",
    null,
    formattedRuns(source).map((run) =>
      composerSchema.text(
        run.text,
        [run.bold && composerSchema.marks.bold.create(), run.italic && composerSchema.marks.italic.create()].filter(
          (mark) => mark !== false,
        ),
      ),
    ),
  );
}

/** Serialize only our existing message subset, never HTML or a new wire format. */
export function composerSource(doc: Node): string {
  return composerEncoding(doc).source;
}

function composerEncoding(doc: Node) {
  const runs: (FormattedRun & { offset: number })[] = [];
  doc.forEach((node, offset) =>
    runs.push({
      text: node.text!,
      offset,
      bold: node.marks.some((mark) => mark.type.name === "bold"),
      italic: node.marks.some((mark) => mark.type.name === "italic"),
    }),
  );
  type StyleRange = { style: "bold" | "italic"; from: number; to: number };
  type Token = { text: string; index?: number; mark?: StyleRange };
  const write = (parts: typeof runs): Token[] => {
    const result: Token[] = [];
    for (let index = 0; index < parts.length;) {
      const first = parts[index];
      if (!first.bold && !first.italic) {
        result.push(...first.text.split("").map((text, index) => ({ text, index: first.offset + index })));
        index++;
        continue;
      }
      // The longer style is the outer pair, keeping nested emphasis contiguous.
      const endOf = (style: "bold" | "italic") => {
        let end = index;
        while (end < parts.length && parts[end][style]) end++;
        return end;
      };
      const style = endOf("bold") >= endOf("italic") ? "bold" : "italic";
      const end = endOf(style);
      const inner = write(parts.slice(index, end).map((run) => ({ ...run, [style]: false })));
      let start = 0;
      let limit = inner.length;
      while (start < limit && /^\s$/u.test(inner[start].text)) start++;
      while (limit > start && /^\s$/u.test(inner[limit - 1].text)) limit--;
      const marker = style === "bold" ? "**" : "_";
      const mark: StyleRange = { style, from: first.offset, to: parts[end - 1].offset + parts[end - 1].text.length };
      result.push(...inner.slice(0, start));
      if (start < limit) result.push({ text: marker, mark }, ...inner.slice(start, limit), { text: marker, mark });
      result.push(...inner.slice(limit));
      index = end;
    }
    return result;
  };
  let source = "";
  const indices: number[] = [];
  const delimiters: (StyleRange & { position: number })[] = [];
  for (const token of write(runs)) {
    if (token.index !== undefined) indices[token.index] = source.length;
    if (token.mark) delimiters.push({ ...token.mark, position: source.length });
    source += token.text;
  }
  return { source, indices, delimiters };
}

export function displayedOffset(source: string, offset: number) {
  return formattedMessage(source).sourceIndices.filter((index) => index < offset).length;
}

export function sourceOffset(source: string, offset: number, end = false) {
  const indices = formattedMessage(source).sourceIndices;
  return end && offset > 0 ? indices[offset - 1] + 1 : (indices[offset] ?? (indices.at(-1) ?? -1) + 1);
}

export function formatComposer(style: "bold" | "italic"): Command {
  return (state, dispatch) => {
    const { from, to } = state.selection;
    if (from === to || splitsLink(state.doc.textContent, from, to)) return false;
    let change: Transaction | undefined;
    toggleMark(composerSchema.marks[style])(state, (transaction) => {
      change = transaction;
    });
    if (!change) return false;
    const source = composerSource(change.doc);
    // Some selections (e.g. part of an identifier in italics) cannot be expressed
    // in the existing wire format. Never show styling that recipients won't see.
    if (Array.from(source).length > 4000 || !composerDocument(source).eq(change.doc)) return false;
    dispatch?.(change);
    return true;
  };
}

/** Typed Markdown shortcuts resolve in the same undo step as the closing marker. */
export function normalizeComposer(transaction: Transaction) {
  const marks = transaction.storedMarks ?? transaction.selection.$head.marks();
  // Typing at the edge of a styled URL can extend it with different marks.
  // Drop that link's styling rather than serializing delimiters into its address.
  let offset = 0;
  for (const part of linkSegments(transaction.doc.textContent)) {
    const end = offset + part.text.length;
    if (part.href) {
      for (const mark of Object.values(composerSchema.marks)) {
        let marked = false;
        let unmarked = false;
        transaction.doc.nodesBetween(offset, end, (node) => {
          if (mark.isInSet(node.marks)) marked = true;
          else unmarked = true;
        });
        if (marked && unmarked) transaction.removeMark(offset, end, mark);
      }
    }
    offset = end;
  }
  let encoding = composerEncoding(transaction.doc);
  for (;;) {
    const visible = new Set(formattedMessage(encoding.source).sourceIndices);
    const invalid = encoding.delimiters.filter((marker) => visible.has(marker.position));
    if (!invalid.length) break;
    // Editing a word boundary can make existing emphasis unrepresentable. Keep
    // the user's text, dropping only that style instead of exposing our markers.
    for (const { from, to, style } of invalid) transaction.removeMark(from, to, composerSchema.marks[style]);
    encoding = composerEncoding(transaction.doc);
  }
  const { source, indices } = encoding;
  const parsed = composerDocument(source);
  if (!parsed.eq(transaction.doc)) {
    const map = (position: number) => displayedOffset(source, position ? indices[position - 1] + 1 : 0);
    const anchor = map(transaction.selection.anchor);
    const head = map(transaction.selection.head);
    transaction.replaceWith(0, transaction.doc.content.size, parsed.content);
    transaction.setSelection(TextSelection.create(transaction.doc, anchor, head));
    transaction.ensureMarks(marks);
  }
  return transaction;
}
