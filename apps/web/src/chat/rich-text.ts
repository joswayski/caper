import { Schema, type Node } from "prosemirror-model";
import { TextSelection, type Command, type Transaction } from "prosemirror-state";
import { toggleMark } from "prosemirror-commands";
import {
  formatSpace,
  formatStyles,
  formattedMessage,
  formattedRuns,
  singleLineFence,
  splitsLink,
  styleBits,
  type FormatStyle,
} from "./formatting.ts";
import { linkSegments } from "./links.ts";

const span = (className: string) => ({
  toDOM: () => ["span", { class: className }, 0] as const,
  parseDOM: [{ tag: `span.${className}` }],
});

// One text block keeps newlines, emoji and mention offsets identical to plain text.
export const composerSchema = new Schema({
  nodes: { doc: { content: "text*", whitespace: "pre" }, text: {} },
  marks: {
    codeBlock: span("chat-code-block-draft"),
    quote: span("chat-quote-draft"),
    bold: { toDOM: () => ["strong", 0], parseDOM: [{ tag: "strong" }, { tag: "b" }] },
    italic: { toDOM: () => ["em", 0], parseDOM: [{ tag: "em" }, { tag: "i" }] },
    underline: { toDOM: () => ["u", 0], parseDOM: [{ tag: "u" }] },
    strikethrough: { toDOM: () => ["s", 0], parseDOM: [{ tag: "s" }] },
    spoiler: span("chat-spoiler-draft"),
    code: { toDOM: () => ["code", { class: "chat-code-draft" }, 0], parseDOM: [{ tag: "code" }] },
  },
});

/** The style bits of a set of marks, as the shared composer cases record a carried style. */
export const markBits = (marks: readonly { type: { name: string } }[]) =>
  marks.reduce((bits, mark) => bits | styleBits[mark.type.name as FormatStyle], 0);

const marksOf = (bits: number) =>
  formatStyles.filter((style) => bits & styleBits[style]).map((style) => composerSchema.marks[style].create());

export function composerDocument(source: string) {
  return composerSchema.node(
    "doc",
    null,
    formattedRuns(source).map((run) =>
      composerSchema.text(
        run.text,
        marksOf(formatStyles.reduce((bits, style) => (run[style] ? bits | styleBits[style] : bits), 0)),
      ),
    ),
  );
}

/** Serialize only our existing message subset, never HTML or a new wire format. */
export function composerSource(doc: Node): string {
  return composerEncoding(doc).source;
}

/** Inline styles in the order the serializer prefers one as the outer pair when two cover the same text.
 * `code` is handled innermost because backticks suppress nested Markdown. */
const inlineOrder = ["bold", "underline", "italic", "strikethrough", "spoiler"] as const;
const markerOf: Record<(typeof inlineOrder)[number] | "code", string> = {
  bold: "**",
  underline: "__",
  italic: "_",
  strikethrough: "~~",
  spoiler: "||",
  code: "`",
};
type StyleRange = { style: FormatStyle; from: number; to: number };
const isWordChar = (ch: string | undefined) => !!ch && /[A-Za-z0-9_@]/.test(ch);

/** The doc's characters (UTF-16 units) and their style bits. */
function characters(doc: Node) {
  let text = "";
  const styles: number[] = [];
  doc.forEach((node) => {
    text += node.text!;
    styles.push(...Array<number>(node.text!.length).fill(markBits(node.marks)));
  });
  return { text, styles };
}

/** The lines of `text` as [start, end, quoted, codeBlock], without their newlines. */
function lines(text: string, styles: number[]) {
  const out: [number, number, boolean, boolean][] = [];
  for (let start = 0; ;) {
    const newline = text.indexOf("\n", start);
    const end = newline < 0 ? text.length : newline;
    const codeBlock = styles.slice(start, end).some((style) => style & styleBits.codeBlock);
    const quoted = !codeBlock && styles.slice(start, end).some((style) => style & styleBits.quote);
    out.push([start, end, quoted, codeBlock]);
    if (newline < 0) return out;
    start = end + 1;
  }
}

function encode(text: string, styles: number[]) {
  type Token = { text: string; index?: number; mark?: StyleRange };
  const write = (start: number, end: number, removed: number): Token[] => {
    const style = (index: number) => styles[index] & ~removed;
    const result: Token[] = [];
    for (let index = start; index < end;) {
      if (!style(index)) {
        result.push({ text: text[index], index });
        index++;
        continue;
      }
      // The longer style is the outer pair, keeping nested emphasis contiguous; `code` is always innermost.
      const endOf = (bit: number) => {
        let limit = index;
        while (limit < end && style(limit) & bit) limit++;
        return limit;
      };
      let name: (typeof inlineOrder)[number] | "code" | undefined;
      let limit = index;
      for (const candidate of inlineOrder) {
        if (!(style(index) & styleBits[candidate])) continue;
        const candidateEnd = endOf(styleBits[candidate]);
        if (!name || candidateEnd > limit) [name, limit] = [candidate, candidateEnd];
      }
      if (!name) {
        name = "code";
        limit = endOf(styleBits.code);
      }
      const inner = write(index, limit, removed | styleBits[name]);
      let first = 0;
      let last = inner.length;
      while (first < last && inner[first].index !== undefined && formatSpace(inner[first].text)) first++;
      while (last > first && inner[last - 1].index !== undefined && formatSpace(inner[last - 1].text)) last--;
      let markerText = markerOf[name];
      if (name === "italic" && first < last) {
        let firstChar: number | undefined;
        for (let k = first; k < last; k++) {
          if (inner[k].index !== undefined) {
            firstChar = inner[k].index;
            break;
          }
        }
        let lastChar: number | undefined;
        for (let k = last - 1; k >= first; k--) {
          if (inner[k].index !== undefined) {
            lastChar = inner[k].index;
            break;
          }
        }
        const nonUnderscore =
          (styleBits.bold | styleBits.strikethrough | styleBits.spoiler | styleBits.code) & ~removed;
        const leftWord =
          firstChar !== undefined &&
          firstChar > start &&
          isWordChar(text[firstChar - 1]) &&
          (styles[firstChar - 1] & nonUnderscore) === 0;
        const rightWord =
          lastChar !== undefined &&
          lastChar + 1 < end &&
          isWordChar(text[lastChar + 1]) &&
          (styles[lastChar + 1] & nonUnderscore) === 0;
        if (leftWord || rightWord) markerText = "*";
      }
      const mark: StyleRange = { style: name, from: index, to: limit };
      result.push(...inner.slice(0, first));
      if (first < last)
        result.push({ text: markerText, mark }, ...inner.slice(first, last), { text: markerText, mark });
      result.push(...inner.slice(last));
      index = limit;
    }
    return result;
  };
  // Quoted lines are encoded alone after `> `; codeBlock lines stay literal; consecutive unquoted lines together.
  const tokens: Token[] = [];
  const split = lines(text, styles);
  const blockMask = styleBits.quote | styleBits.codeBlock;
  for (let index = 0; index < split.length;) {
    const [start, end, quoted, codeBlock] = split[index];
    if (codeBlock) {
      for (let i = start; i < end; i++) tokens.push({ text: text[i], index: i });
      index++;
    } else if (quoted) {
      tokens.push({ text: "> ", mark: { style: "quote", from: start, to: end } }, ...write(start, end, blockMask));
      index++;
    } else {
      let last = index;
      while (last + 1 < split.length && !split[last + 1][2] && !split[last + 1][3]) last++;
      tokens.push(...write(start, split[last][1], blockMask));
      index = last + 1;
    }
    if (index < split.length) tokens.push({ text: "\n", index: split[index - 1][1] });
  }
  let source = "";
  const indices: number[] = [];
  const delimiters: (StyleRange & { position: number })[] = [];
  for (const token of tokens) {
    if (token.index !== undefined) indices[token.index] = source.length;
    if (token.mark) delimiters.push({ ...token.mark, position: source.length });
    source += token.text;
  }
  return { source, indices, delimiters };
}

function composerEncoding(doc: Node) {
  const { text, styles } = characters(doc);
  return encode(text, styles);
}

export function displayedOffset(source: string, offset: number) {
  return formattedMessage(source).sourceIndices.filter((index) => index < offset).length;
}

export function sourceOffset(source: string, offset: number, end = false) {
  const indices = formattedMessage(source).sourceIndices;
  return end && offset > 0 ? indices[offset - 1] + 1 : (indices[offset] ?? (indices.at(-1) ?? -1) + 1);
}

function docFromStyles(text: string, styles: number[]) {
  const nodes: Node[] = [];
  for (let start = 0; start < text.length;) {
    let end = start + 1;
    while (end < text.length && styles[end] === styles[start]) end++;
    nodes.push(composerSchema.text(text.slice(start, end), marksOf(styles[start])));
    start = end;
  }
  return composerSchema.node("doc", null, nodes);
}

/** `> ` for each non-blank line the selection touches; newlines inside a quote are quoted, at its edges plain. */
function toggleQuote(doc: Node, from: number, to: number) {
  const { text, styles } = characters(doc);
  const split = lines(text, styles);
  const touched = split.filter(
    ([start, end, , codeBlock]) => !codeBlock && start < to && end >= from && /\S/.test(text.slice(start, end)),
  );
  if (!touched.length) return undefined;
  const add = !touched.some(([, , quoted]) => quoted);
  const quoted = split.map(([, , quoted]) => quoted);
  for (const line of touched) {
    const [start, end] = line;
    quoted[split.indexOf(line)] = add;
    for (let index = start; index < end; index++)
      styles[index] = add ? styles[index] | styleBits.quote : styles[index] & ~styleBits.quote;
  }
  for (let line = 1; line < split.length; line++) {
    const newline = split[line][0] - 1;
    if (quoted[line - 1] || quoted[line]) styles[newline] = quoted[line - 1] && quoted[line] ? styleBits.quote : 0;
    else if (styles[newline] & styleBits.quote) styles[newline] = 0;
  }
  return docFromStyles(text, styles);
}

/** Wraps the selection in a fenced code block, or unwraps the enclosing code block when already inside one. */
function toggleCodeBlock(doc: Node, from: number, to: number) {
  const { text, styles } = characters(doc);
  let allCodeBlock = true;
  let anyCodeBlock = false;
  for (let i = from; i < to; i++) {
    if (styles[i] & styleBits.codeBlock) anyCodeBlock = true;
    else allCodeBlock = false;
  }
  if (anyCodeBlock && !allCodeBlock) return undefined;
  if (allCodeBlock) {
    let blkStart = from;
    while (blkStart > 0 && styles[blkStart - 1] & styleBits.codeBlock) blkStart--;
    let blkEnd = to;
    while (blkEnd < styles.length && styles[blkEnd] & styleBits.codeBlock) blkEnd++;
    let innerStart: number;
    let innerEnd: number;
    const single = singleLineFence(text, blkStart, blkEnd);
    if (single) {
      [innerStart, innerEnd] = single;
    } else {
      const firstNl = text.indexOf("\n", blkStart);
      const lastNl = text.lastIndexOf("\n", blkEnd - 1);
      if (firstNl < 0 || lastNl < 0 || firstNl >= lastNl) return undefined;
      innerStart = firstNl + 1;
      innerEnd = lastNl;
    }
    const body = text.slice(innerStart, innerEnd);
    if (!body) return undefined;
    const nextText = text.slice(0, blkStart) + body + text.slice(blkEnd);
    const nextStyles = [...styles.slice(0, blkStart), ...Array<number>(body.length).fill(0), ...styles.slice(blkEnd)];
    const clampedFrom = Math.min(Math.max(from, innerStart), innerEnd) - innerStart;
    const clampedTo = Math.min(Math.max(to, innerStart), innerEnd) - innerStart;
    const selFrom = clampedFrom < clampedTo ? blkStart + clampedFrom : blkStart;
    const selTo = clampedFrom < clampedTo ? blkStart + clampedTo : blkStart + body.length;
    return { doc: docFromStyles(nextText, nextStyles), selFrom, selTo };
  }
  if (!/\S/.test(text.slice(from, to))) return undefined;
  const needLeadingNl = from > 0 && text[from - 1] !== "\n";
  const needTrailingNl = to < text.length && text[to] !== "\n";
  const prefix = (needLeadingNl ? "\n" : "") + "```\n";
  const suffix = "\n```" + (needTrailingNl ? "\n" : "");
  const prefixStyles = [...(needLeadingNl ? [0] : []), ...Array<number>(4).fill(styleBits.codeBlock)];
  const suffixStyles = [...Array<number>(4).fill(styleBits.codeBlock), ...(needTrailingNl ? [0] : [])];
  const nextText = text.slice(0, from) + prefix + text.slice(from, to) + suffix + text.slice(to);
  const nextStyles = [
    ...styles.slice(0, from),
    ...prefixStyles,
    ...Array<number>(to - from).fill(styleBits.codeBlock),
    ...suffixStyles,
    ...styles.slice(to),
  ];
  const selFrom = from + prefix.length;
  const selTo = selFrom + (to - from);
  return { doc: docFromStyles(nextText, nextStyles), selFrom, selTo };
}

export function formatComposer(style: FormatStyle): Command {
  return (state, dispatch) => {
    const { from, to } = state.selection;
    if (from === to || splitsLink(state.doc.textContent, from, to)) return false;
    if (style === "codeBlock") {
      const result = toggleCodeBlock(state.doc, from, to);
      if (!result) return false;
      const source = composerSource(result.doc);
      if (Array.from(source).length > 4000 || !composerDocument(source).eq(result.doc)) return false;
      const change = state.tr.replaceWith(0, state.doc.content.size, result.doc.content);
      change.setSelection(TextSelection.create(change.doc, result.selFrom, result.selTo));
      dispatch?.(change);
      return true;
    }
    if (state.doc.rangeHasMark(from, to, composerSchema.marks.codeBlock)) return false;
    if (style === "code" && !state.doc.rangeHasMark(from, to, composerSchema.marks.code)) {
      const selected = state.doc.textBetween(from, to);
      if (selected.trim().includes("\n")) return false;
    }
    let change: Transaction | undefined;
    if (style === "quote") {
      const doc = toggleQuote(state.doc, from, to);
      if (doc) change = state.tr.replaceWith(0, state.doc.content.size, doc.content);
    } else
      toggleMark(composerSchema.marks[style])(state, (transaction) => {
        change = transaction;
      });
    if (!change) return false;
    const source = composerSource(change.doc);
    // Some selections cannot be expressed in the existing wire format.
    // Never show styling that recipients won't see.
    if (Array.from(source).length > 4000 || !composerDocument(source).eq(change.doc)) return false;
    if (style === "quote") change.setSelection(TextSelection.create(change.doc, from, to));
    dispatch?.(change);
    return true;
  };
}

/** Whether text typed at the caret is quoted: its line is, or the caret starts an empty line after a quoted one. */
function quotedAt(doc: Node, head: number) {
  const text = doc.textContent;
  let start = text.lastIndexOf("\n", head - 1) + 1;
  let end = text.indexOf("\n", head);
  if (end < 0) end = text.length;
  if (start === end) {
    if (head === 0) return false;
    end = head - 1;
    start = text.lastIndexOf("\n", end - 1) + 1;
  }
  return start < end && doc.rangeHasMark(start, end, composerSchema.marks.quote);
}

/** Typed Markdown shortcuts resolve in the same undo step as the closing marker. */
export function normalizeComposer(transaction: Transaction) {
  let marks = transaction.storedMarks ?? transaction.selection.$head.marks();
  // Typing at the edge of a styled URL can extend it with different marks.
  // Drop that link's styling rather than serializing delimiters into its address.
  let offset = 0;
  for (const part of linkSegments(transaction.doc.textContent)) {
    const end = offset + part.text.length;
    if (part.href) {
      for (const mark of Object.values(composerSchema.marks)) {
        if (mark === composerSchema.marks.codeBlock) continue;
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
    // A quote belongs to lines: a newline typed in one continues it, and a second leaves it.
    const quote = composerSchema.marks.quote;
    marks = quotedAt(transaction.doc, head) ? quote.create().addToSet(marks) : quote.removeFromSet(marks);
    transaction.ensureMarks(marks);
  }
  return transaction;
}
